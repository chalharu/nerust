/// Android-relevant settings subset and JNI dialog bridge.
///
/// Only the fields that make sense on a mobile/touch device are exposed.
/// All persistence and validation remain on the Rust side; Kotlin merely
/// presents the choices and returns the user's selections.
use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::refs::Global;
use jni::sys::jobject;
use jni::{JavaVM, jni_sig, jni_str};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::nes::{NesSettings, NesVideoFilter};
use nerust_gui_settings::shared::SystemSettings;
use nerust_input_schema::SystemId;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

// ---------------------------------------------------------------------------
// Choice constants
// ---------------------------------------------------------------------------

const VOLUME_MIN: u8 = 0;
const VOLUME_MAX: u8 = 100;
const LATENCY_MIN: u16 = 10;
const LATENCY_MAX: u16 = 200;
fn sample_rate_choices() -> &'static [u32] {
    static CHOICES: OnceLock<Vec<u32>> = OnceLock::new();
    CHOICES.get_or_init(|| {
        let rates = nerust_gui_shell::settings::nes::audio_registry().supported_rates();
        if rates.is_empty() {
            vec![44_100, 48_000]
        } else {
            rates.to_vec()
        }
    })
}

/// All four variants in declaration order (matches `NesVideoFilter`'s natural ordering).
const FILTER_CHOICES: &[NesVideoFilter] = &[
    NesVideoFilter::None,
    NesVideoFilter::NtscComposite,
    NesVideoFilter::NtscSVideo,
    NesVideoFilter::NtscRgb,
];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// The Android-relevant subset of the full settings snapshot.
///
/// Derived from [`SettingsSnapshot`] on the way in; applied back via
/// [`AndroidSettings::apply_to_snapshot`] on the way out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AndroidSettings {
    pub audio_muted: bool,
    pub master_volume_percent: u8,
    pub latency_ms: u16,
    pub sample_rate: u32,
    pub vsync: bool,
    pub nes_filter: NesVideoFilter,
}

impl AndroidSettings {
    /// Extract Android-relevant fields from the full settings snapshot.
    pub(crate) fn from_snapshot(snapshot: &SettingsSnapshot) -> Self {
        let nes_filter = snapshot
            .shared
            .systems
            .get(&SystemId::Nes)
            .map(|s| match s {
                SystemSettings::Nes(n) => n.video.filter,
            })
            .unwrap_or_default();

        Self {
            audio_muted: snapshot.local.audio.muted,
            master_volume_percent: snapshot.local.audio.master_volume_percent,
            latency_ms: snapshot.local.audio.latency_ms,
            sample_rate: snapshot.local.audio.sample_rate,
            vsync: snapshot.local.video.presentation.vsync,
            nes_filter,
        }
    }

    /// Write the Android-relevant fields back into a full settings snapshot.
    ///
    /// Fields not exposed by the Android UI are left untouched.
    pub(crate) fn apply_to_snapshot(&self, snapshot: &mut SettingsSnapshot) {
        snapshot.local.audio.muted = self.audio_muted;
        snapshot.local.audio.master_volume_percent = self.master_volume_percent;
        snapshot.local.audio.latency_ms = self.latency_ms;
        snapshot.local.audio.sample_rate = self.sample_rate;
        snapshot.local.video.presentation.vsync = self.vsync;

        let system = snapshot
            .shared
            .systems
            .entry(SystemId::Nes)
            .or_insert_with(|| SystemSettings::Nes(NesSettings::default()));
        let SystemSettings::Nes(nes) = system;
        nes.video.filter = self.nes_filter;
    }

    // -----------------------------------------------------------------------
    // Dialog encoding
    // -----------------------------------------------------------------------

    /// Stable setting keys sent to Kotlin (also used for decoding the result).
    pub(crate) fn dialog_keys() -> &'static [&'static str] {
        &[
            "audio_muted",
            "master_volume",
            "latency_ms",
            "sample_rate",
            "vsync",
            "nes_filter",
        ]
    }

    /// Human-readable labels, one per key, in the same order.
    pub(crate) fn dialog_labels() -> &'static [&'static str] {
        &[
            "Mute",
            "Volume",
            "Audio Latency (ms)",
            "Sample Rate (Hz)",
            "VSync",
            "NES Video Filter",
        ]
    }

    /// Tab-separated choice labels, one string per setting, in key order.
    pub(crate) fn dialog_choices() -> Vec<String> {
        vec![
            "Off\tOn".to_string(),
            join_tab_labels((VOLUME_MIN..=VOLUME_MAX).map(|value| format!("{value}%"))),
            join_tab_labels((LATENCY_MIN..=LATENCY_MAX).map(|value| format!("{value} ms"))),
            join_tab_labels(
                sample_rate_choices()
                    .iter()
                    .map(|value| format!("{value} Hz")),
            ),
            "Off\tOn".to_string(),
            "None\tNTSC Composite\tNTSC S-Video\tNTSC RGB".to_string(),
        ]
    }

    /// Index of the current choice for each setting, in key order, as strings.
    pub(crate) fn current_indices(&self) -> Vec<String> {
        let volume_idx = usize::from(self.master_volume_percent.min(VOLUME_MAX));
        let latency_idx =
            usize::from(self.latency_ms.clamp(LATENCY_MIN, LATENCY_MAX) - LATENCY_MIN);
        let choices = sample_rate_choices();
        let sample_rate_idx = choices
            .iter()
            .position(|&v| v == self.sample_rate)
            .unwrap_or(choices.len().saturating_sub(1)); // default: highest rate
        let filter_idx = FILTER_CHOICES
            .iter()
            .position(|&v| v == self.nes_filter)
            .unwrap_or_else(|| {
                log::warn!("NES video filter is not representable on Android, defaulting to None");
                0
            });

        vec![
            (self.audio_muted as usize).to_string(),
            volume_idx.to_string(),
            latency_idx.to_string(),
            sample_rate_idx.to_string(),
            (self.vsync as usize).to_string(),
            filter_idx.to_string(),
        ]
    }

    /// Build an `AndroidSettings` from a comma-separated list of choice indices
    /// (as returned by the Kotlin callback).
    ///
    /// Returns `None` if the string is malformed or any index is out of range.
    pub(crate) fn from_choice_indices(raw: &str) -> Option<Self> {
        let indices: Vec<usize> = raw
            .split(',')
            .map(|s| s.trim().parse::<usize>().ok())
            .collect::<Option<_>>()?;

        if indices.len() != Self::dialog_keys().len() {
            return None;
        }

        let audio_muted = match indices[0] {
            0 => false,
            1 => true,
            _ => return None,
        };
        let master_volume_percent = u8::try_from(indices[1])
            .ok()
            .filter(|value| *value <= VOLUME_MAX)?;
        let latency_ms = u16::try_from(indices[2])
            .ok()
            .filter(|value| *value <= LATENCY_MAX - LATENCY_MIN)
            .map(|value| value + LATENCY_MIN)?;
        let sample_rate = *sample_rate_choices().get(indices[3])?;
        let vsync = match indices[4] {
            0 => false,
            1 => true,
            _ => return None,
        };
        let nes_filter = *FILTER_CHOICES.get(indices[5])?;

        Some(Self {
            audio_muted,
            master_volume_percent,
            latency_ms,
            sample_rate,
            vsync,
            nes_filter,
        })
    }
}

fn join_tab_labels(values: impl IntoIterator<Item = String>) -> String {
    let mut labels = values.into_iter();
    let mut joined = labels.next().unwrap_or_default();
    for value in labels {
        joined.push('\t');
        joined.push_str(&value);
    }
    joined
}

// ---------------------------------------------------------------------------
// State machine (mirrors the library / picker pattern)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsDialogResult {
    /// The user dismissed the dialog without saving.
    Dismissed,
    /// The user saved settings; the value encodes the comma-separated indices.
    Applied(String),
}

static SETTINGS_RESULT: Mutex<Option<SettingsDialogResult>> = Mutex::new(None);
static SETTINGS_WAKER: Mutex<Option<AndroidAppWaker>> = Mutex::new(None);
static SETTINGS_REQUEST_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Pre-marshalled settings data for synchronous dialog display from JNI callbacks.
static CACHED_SETTINGS: Mutex<CachedSettingsData> = Mutex::new(CachedSettingsData::empty());

struct CachedSettingsData {
    keys: Vec<String>,
    labels: Vec<String>,
    choices: Vec<String>,
    current_indices: Vec<String>,
}

impl CachedSettingsData {
    const fn empty() -> Self {
        Self {
            keys: Vec::new(),
            labels: Vec::new(),
            choices: Vec::new(),
            current_indices: Vec::new(),
        }
    }
}

/// Update cached settings so `show_settings_dialog_sync` can present current data.
pub(crate) fn update_cached_settings(current: &AndroidSettings) {
    let keys: Vec<String> = AndroidSettings::dialog_keys()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let labels: Vec<String> = AndroidSettings::dialog_labels()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let choices = AndroidSettings::dialog_choices();
    let current_indices = current.current_indices();
    *CACHED_SETTINGS
        .lock()
        .expect("cached settings mutex poisoned") = CachedSettingsData {
        keys,
        labels,
        choices,
        current_indices,
    };
}

/// Show the settings dialog synchronously from a JNI callback running on the
/// Java main thread.  Returns `Ok(false)` if a dialog is already in flight.
pub(crate) fn show_settings_dialog_sync(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
) -> Result<bool, String> {
    if SETTINGS_REQUEST_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return Ok(false);
    }
    let cached = CACHED_SETTINGS
        .lock()
        .expect("cached settings mutex poisoned");
    let keys = cached.keys.clone();
    let labels = cached.labels.clone();
    let choices = cached.choices.clone();
    let current_indices = cached.current_indices.clone();
    drop(cached);

    if let Err(error) =
        show_settings_with_env(env, activity, &keys, &labels, &choices, &current_indices)
    {
        SETTINGS_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
        return Err(error);
    }
    Ok(true)
}

pub(crate) fn bind_app(app: &AndroidApp) {
    *SETTINGS_WAKER
        .lock()
        .expect("settings waker mutex poisoned") = Some(app.create_waker());
    *SETTINGS_RESULT
        .lock()
        .expect("settings result mutex poisoned") = None;
    SETTINGS_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
}

pub(crate) fn reset() {
    *SETTINGS_RESULT
        .lock()
        .expect("settings result mutex poisoned") = None;
    SETTINGS_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
}

pub(crate) fn take_result() -> Option<SettingsDialogResult> {
    SETTINGS_RESULT
        .lock()
        .expect("settings result mutex poisoned")
        .take()
}

/// Request that the Android side show the settings dialog.
///
/// Returns `Ok(false)` when a dialog is already in flight (idempotent guard).
pub(crate) fn request_show_settings_dialog(
    app: &AndroidApp,
    current: &AndroidSettings,
) -> Result<bool, String> {
    if SETTINGS_REQUEST_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return Ok(false);
    }

    let keys: Vec<String> = AndroidSettings::dialog_keys()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let labels: Vec<String> = AndroidSettings::dialog_labels()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let choices = AndroidSettings::dialog_choices();
    let current_indices = current.current_indices();

    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = show_settings_on_java_main_thread(
            &callback_app,
            &keys,
            &labels,
            &choices,
            &current_indices,
        ) {
            log::error!("{error}");
            SETTINGS_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
            wake_main_thread();
        }
    }));
    Ok(true)
}

fn show_settings_on_java_main_thread(
    app: &AndroidApp,
    keys: &[String],
    labels: &[String],
    choices: &[String],
    current_indices: &[String],
) -> Result<(), String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    vm.attach_current_thread(|env| {
        let activity_raw = app.activity_as_ptr() as jobject;
        let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
        show_settings_with_env_inner(
            env,
            activity.as_ref(),
            keys,
            labels,
            choices,
            current_indices,
        )
    })
    .map_err(|error| format!("failed to show Android settings dialog: {error:?}"))
}

fn show_settings_with_env(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    keys: &[String],
    labels: &[String],
    choices: &[String],
    current_indices: &[String],
) -> Result<(), String> {
    let n = keys.len();
    env.with_local_frame(4 + n * 4 + 8, |env| {
        show_settings_with_env_inner(env, activity, keys, labels, choices, current_indices)
    })
    .map_err(|error| format!("failed to show Android settings dialog: {error:?}"))
}

fn show_settings_with_env_inner(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    keys: &[String],
    labels: &[String],
    choices: &[String],
    current_indices: &[String],
) -> Result<(), jni::errors::Error> {
    let string_class = env.find_class(jni_str!("java/lang/String"))?;

    let mut make_string_array = |items: &[String]| -> Result<JObjectArray<'_>, jni::errors::Error> {
        let arr = env.new_object_array(items.len() as _, &string_class, JObject::null())?;
        for (i, s) in items.iter().enumerate() {
            let js = env.new_string(s.as_str())?;
            arr.set_element(env, i, &js)?;
        }
        Ok(arr)
    };

    let keys_arr = make_string_array(keys)?;
    let labels_arr = make_string_array(labels)?;
    let choices_arr = make_string_array(choices)?;
    let current_arr = make_string_array(current_indices)?;

    env.call_method(
        activity,
        jni_str!("showSettingsDialog"),
        jni_sig!("([Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V"),
        &[
            JValue::Object(keys_arr.as_ref()),
            JValue::Object(labels_arr.as_ref()),
            JValue::Object(choices_arr.as_ref()),
            JValue::Object(current_arr.as_ref()),
        ],
    )?;
    Ok(())
}

fn publish_result(result: SettingsDialogResult) {
    *SETTINGS_RESULT
        .lock()
        .expect("settings result mutex poisoned") = Some(result);
    SETTINGS_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
    wake_main_thread();
}

fn wake_main_thread() {
    if let Some(waker) = SETTINGS_WAKER
        .lock()
        .expect("settings waker mutex poisoned")
        .clone()
    {
        waker.wake();
    }
}

// ---------------------------------------------------------------------------
// JNI callback – invoked by `MainActivity.onSettingsDialogResult`
// ---------------------------------------------------------------------------
//
// * `result == null`  → dialog was dismissed
// * `result` is a comma-separated string of choice indices, e.g. "0,4,1,1,1,1"

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onSettingsDialogResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    result: JString<'_>,
) {
    match env
        .with_env(|env| -> jni::errors::Result<SettingsDialogResult> {
            if result.is_null() {
                Ok(SettingsDialogResult::Dismissed)
            } else {
                let result = result.try_to_string(env)?;
                Ok(SettingsDialogResult::Applied(result))
            }
        })
        .into_outcome()
    {
        jni::Outcome::Ok(r) => publish_result(r),
        jni::Outcome::Err(error) => {
            log::error!("failed to decode Android settings dialog result: {error:?}");
            publish_result(SettingsDialogResult::Dismissed);
        }
        jni::Outcome::Panic(_) => {
            log::error!("Android settings dialog callback panicked");
            publish_result(SettingsDialogResult::Dismissed);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_settings::app_state::DesktopAppState;
    use nerust_gui_settings::local::HostBackendLocalSettings;
    use nerust_gui_settings::nes::{NesSettings, NesVideoFilter};
    use nerust_gui_settings::shared::{DesktopSharedSettings, SystemSettings};
    use nerust_input_schema::SystemId;

    fn default_snapshot() -> SettingsSnapshot {
        let mut shared = DesktopSharedSettings::default();
        shared
            .systems
            .insert(SystemId::Nes, SystemSettings::Nes(NesSettings::default()));
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    #[test]
    fn round_trips_default_snapshot() {
        let snapshot = default_snapshot();
        let android = AndroidSettings::from_snapshot(&snapshot);
        let mut out = default_snapshot();
        android.apply_to_snapshot(&mut out);
        // The round-trip should not change anything when starting from defaults.
        assert_eq!(out.local.audio.muted, snapshot.local.audio.muted);
        assert_eq!(
            out.local.audio.master_volume_percent,
            snapshot.local.audio.master_volume_percent
        );
        assert_eq!(out.local.audio.latency_ms, snapshot.local.audio.latency_ms);
        assert_eq!(
            out.local.audio.sample_rate,
            snapshot.local.audio.sample_rate
        );
        assert_eq!(
            out.local.video.presentation.vsync,
            snapshot.local.video.presentation.vsync
        );
    }

    #[test]
    fn from_snapshot_extracts_nes_filter() {
        let mut snapshot = default_snapshot();
        let SystemSettings::Nes(nes) = snapshot.shared.systems.get_mut(&SystemId::Nes).unwrap();
        nes.video.filter = NesVideoFilter::NtscSVideo;

        let android = AndroidSettings::from_snapshot(&snapshot);
        assert_eq!(android.nes_filter, NesVideoFilter::NtscSVideo);
    }

    #[test]
    fn apply_to_snapshot_writes_all_fields() {
        let android = AndroidSettings {
            audio_muted: true,
            master_volume_percent: 50,
            latency_ms: 75,
            sample_rate: 44_100,
            vsync: false,
            nes_filter: NesVideoFilter::NtscRgb,
        };

        let mut snapshot = default_snapshot();
        android.apply_to_snapshot(&mut snapshot);

        assert!(snapshot.local.audio.muted);
        assert_eq!(snapshot.local.audio.master_volume_percent, 50);
        assert_eq!(snapshot.local.audio.latency_ms, 75);
        assert_eq!(snapshot.local.audio.sample_rate, 44_100);
        assert!(!snapshot.local.video.presentation.vsync);
        let SystemSettings::Nes(nes) = snapshot.shared.systems.get(&SystemId::Nes).unwrap();
        assert_eq!(nes.video.filter, NesVideoFilter::NtscRgb);
    }

    #[test]
    fn current_indices_matches_defaults() {
        let snapshot = default_snapshot();
        let android = AndroidSettings::from_snapshot(&snapshot);
        let indices = android.current_indices();
        // Default: not muted → 0; volume 100% → index 100; latency 50 ms → index 40;
        // sample rate 48000 → index 2; vsync on → 1; NtscComposite → index 1
        assert_eq!(indices, vec!["0", "100", "40", "2", "1", "1"]);
    }

    #[test]
    fn from_choice_indices_round_trips() {
        let original = AndroidSettings {
            audio_muted: true,
            master_volume_percent: 25,
            latency_ms: 100,
            sample_rate: 44_100,
            vsync: false,
            nes_filter: NesVideoFilter::NtscSVideo,
        };

        let mut snapshot = default_snapshot();
        original.apply_to_snapshot(&mut snapshot);
        let recovered = AndroidSettings::from_snapshot(&snapshot);
        let indices_str = recovered.current_indices().join(",");

        let parsed = AndroidSettings::from_choice_indices(&indices_str).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_choice_indices_round_trips_non_default_audio_values() {
        let original = AndroidSettings {
            audio_muted: false,
            master_volume_percent: 83,
            latency_ms: 37,
            sample_rate: 22_050,
            vsync: true,
            nes_filter: NesVideoFilter::None,
        };

        let indices_str = original.current_indices().join(",");
        let parsed = AndroidSettings::from_choice_indices(&indices_str).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_choice_indices_rejects_out_of_range() {
        assert!(AndroidSettings::from_choice_indices("0,101,1,1,1,1").is_none());
        assert!(AndroidSettings::from_choice_indices("0,4,191,1,1,1").is_none());
        assert!(AndroidSettings::from_choice_indices("2,4,1,1,1,1").is_none());
        assert!(AndroidSettings::from_choice_indices("0,4,1,1,2,1").is_none());
    }

    #[test]
    fn dialog_choices_cover_full_android_audio_range() {
        let choices = AndroidSettings::dialog_choices();
        let volume_choices: Vec<_> = choices[1].split('\t').collect();
        let latency_choices: Vec<_> = choices[2].split('\t').collect();
        let sample_rate_choices: Vec<_> = choices[3].split('\t').collect();

        assert_eq!(volume_choices.first(), Some(&"0%"));
        assert_eq!(volume_choices.last(), Some(&"100%"));
        assert_eq!(volume_choices.len(), 101);

        assert_eq!(latency_choices.first(), Some(&"10 ms"));
        assert_eq!(latency_choices.last(), Some(&"200 ms"));
        assert_eq!(latency_choices.len(), 191);

        assert!(
            !sample_rate_choices.is_empty(),
            "sample rate choices should be non-empty"
        );
        for choice in &sample_rate_choices {
            let Some(rate_str) = choice.strip_suffix(" Hz") else {
                panic!("sample rate choice '{choice}' must end with ' Hz'");
            };
            let rate: u32 = rate_str
                .parse()
                .expect("sample rate must be a valid integer");
            assert!(
                (1..=192_000).contains(&rate),
                "sample rate {rate} must be within 1..=192000"
            );
        }
    }

    #[test]
    fn from_choice_indices_rejects_wrong_length() {
        assert!(AndroidSettings::from_choice_indices("0,4,1,1,1").is_none()); // too short
        assert!(AndroidSettings::from_choice_indices("0,4,1,1,1,1,0").is_none()); // too long
    }

    #[test]
    fn dialog_arrays_are_consistent_length() {
        let n = AndroidSettings::dialog_keys().len();
        assert_eq!(AndroidSettings::dialog_labels().len(), n);
        assert_eq!(AndroidSettings::dialog_choices().len(), n);
        let snapshot = default_snapshot();
        let android = AndroidSettings::from_snapshot(&snapshot);
        assert_eq!(android.current_indices().len(), n);
    }
}
