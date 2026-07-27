use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Android-relevant settings subset and JNI dialog bridge.
///
/// Only the fields that make sense on a mobile/touch device are exposed.
/// All persistence and validation remain on the Rust side; Kotlin merely
/// presents the choices and returns the user's selections.
use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::{JavaVM, jni_sig, jni_str, refs::Global, sys::jobject};
use nerust_core_traits::{
    factory::{
        FactoryError,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind},
    },
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::factory::{apply_settings_choice, resolve_label, settings_view};
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

// ---------------------------------------------------------------------------
// Choice constants
// ---------------------------------------------------------------------------

const VOLUME_MIN: u8 = 0;
const VOLUME_MAX: u8 = 100;
const LATENCY_MIN: u16 = 10;
const LATENCY_MAX: u16 = 200;
const SAMPLE_RATE_CHOICES: &[u32] = &[44_100, 48_000];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// The Android-relevant subset of the full settings snapshot.
///
/// Derived from [`SettingsSnapshot`] on the way in; applied back via
/// [`AndroidSettings::apply_to_snapshot`] on the way out.
///
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AndroidSettings {
    pub audio_muted: bool,
    pub master_volume_percent: u8,
    pub latency_ms: u16,
    pub sample_rate: u32,
    pub vsync: bool,
    system_choices: Vec<AndroidSystemChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AndroidSystemChoice {
    system_id: Box<dyn SystemId>,
    field_id: SystemSettingsFieldId,
    label: String,
    selected: SystemSettingsChoiceId,
    options: Vec<(SystemSettingsChoiceId, String)>,
}

impl AndroidSettings {
    /// Extract Android-relevant fields from the full settings snapshot.
    pub(crate) fn from_snapshot(snapshot: &SettingsSnapshot, registry: &SystemRegistry) -> Self {
        let language = snapshot.shared.general.language;
        let system_choices = registry
            .all()
            .iter()
            .flat_map(|factory| {
                let system_id = factory.system_id();
                let view = settings_view(snapshot, system_id.as_ref());
                factory
                    .settings_page(&view)
                    .fields
                    .iter()
                    .map(|field| {
                        let SystemSettingsFieldKind::Choice { selected, options } = &field.kind;
                        AndroidSystemChoice {
                            system_id: system_id.clone(),
                            field_id: field.id.clone(),
                            label: resolve_label(field.label_id, language, factory.as_ref()),
                            selected: selected.clone(),
                            options: options
                                .iter()
                                .map(|option| {
                                    (
                                        option.id.clone(),
                                        resolve_label(option.label_id, language, factory.as_ref()),
                                    )
                                })
                                .collect(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        Self {
            audio_muted: snapshot.local.audio.muted,
            master_volume_percent: snapshot.local.audio.master_volume_percent,
            latency_ms: snapshot.local.audio.latency_ms,
            sample_rate: snapshot.local.audio.sample_rate,
            vsync: snapshot.local.video.presentation.vsync,
            system_choices,
        }
    }

    /// Write the Android-relevant fields back into a full settings snapshot.
    ///
    /// Fields not exposed by the Android UI are left untouched.
    pub(crate) fn apply_to_snapshot(
        &self,
        snapshot: &mut SettingsSnapshot,
        registry: &SystemRegistry,
    ) -> Result<(), FactoryError> {
        snapshot.local.audio.muted = self.audio_muted;
        snapshot.local.audio.master_volume_percent = self.master_volume_percent;
        snapshot.local.audio.latency_ms = self.latency_ms;
        snapshot.local.audio.sample_rate = self.sample_rate;
        snapshot.local.video.presentation.vsync = self.vsync;

        for choice in &self.system_choices {
            let factory = registry
                .find_by_id(choice.system_id.as_ref())
                .ok_or(FactoryError::InvalidSettings)?;
            apply_settings_choice(
                factory.as_ref(),
                snapshot,
                &choice.field_id,
                &choice.selected,
            )?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Dialog encoding
    // -----------------------------------------------------------------------

    /// Stable setting keys sent to Kotlin (also used for decoding the result).
    pub(crate) fn dialog_keys(&self) -> Vec<String> {
        let mut keys = vec![
            "audio_muted".to_string(),
            "master_volume".to_string(),
            "latency_ms".to_string(),
            "sample_rate".to_string(),
            "vsync".to_string(),
        ];
        keys.extend(
            self.system_choices
                .iter()
                .map(|choice| format!("system.{}.{}", choice.system_id, choice.field_id.as_str())),
        );
        keys
    }

    /// Human-readable labels, one per key, in the same order.
    pub(crate) fn dialog_labels(&self) -> Vec<String> {
        let mut labels = vec![
            "Mute".to_string(),
            "Volume".to_string(),
            "Audio Latency (ms)".to_string(),
            "Sample Rate (Hz)".to_string(),
            "VSync".to_string(),
        ];
        labels.extend(
            self.system_choices
                .iter()
                .map(|choice| choice.label.clone()),
        );
        labels
    }

    /// Tab-separated choice labels, one string per setting, in key order.
    pub(crate) fn dialog_choices(&self) -> Vec<String> {
        let mut choices = vec![
            "Off\tOn".to_string(),
            join_tab_labels((VOLUME_MIN..=VOLUME_MAX).map(|value| format!("{value}%"))),
            join_tab_labels((LATENCY_MIN..=LATENCY_MAX).map(|value| format!("{value} ms"))),
            join_tab_labels(
                SAMPLE_RATE_CHOICES
                    .iter()
                    .map(|value| format!("{value} Hz")),
            ),
            "Off\tOn".to_string(),
        ];
        choices.extend(
            self.system_choices.iter().map(|choice| {
                join_tab_labels(choice.options.iter().map(|(_, label)| label.clone()))
            }),
        );
        choices
    }

    /// Index of the current choice for each setting, in key order, as strings.
    pub(crate) fn current_indices(&self) -> Vec<String> {
        let volume_idx = usize::from(self.master_volume_percent.min(VOLUME_MAX));
        let latency_idx =
            usize::from(self.latency_ms.clamp(LATENCY_MIN, LATENCY_MAX) - LATENCY_MIN);
        let sample_rate_idx = SAMPLE_RATE_CHOICES
            .iter()
            .position(|&v| v == self.sample_rate)
            .unwrap_or(SAMPLE_RATE_CHOICES.len().saturating_sub(1)); // default: highest rate
        let mut indices = vec![
            (self.audio_muted as usize).to_string(),
            volume_idx.to_string(),
            latency_idx.to_string(),
            sample_rate_idx.to_string(),
            (self.vsync as usize).to_string(),
        ];
        indices.extend(self.system_choices.iter().map(|choice| {
            choice
                .options
                .iter()
                .position(|(id, _)| id == &choice.selected)
                .unwrap_or_default()
                .to_string()
        }));
        indices
    }

    /// Build an `AndroidSettings` from a comma-separated list of choice indices
    /// (as returned by the Kotlin callback).
    ///
    /// Returns `None` if the string is malformed or any index is out of range.
    pub(crate) fn from_choice_indices(raw: &str, current: &Self) -> Option<Self> {
        let indices: Vec<usize> = raw
            .split(',')
            .map(|s| s.trim().parse::<usize>().ok())
            .collect::<Option<_>>()?;

        if indices.len() != current.dialog_keys().len() {
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
        let sample_rate = *SAMPLE_RATE_CHOICES.get(indices[3])?;
        let vsync = match indices[4] {
            0 => false,
            1 => true,
            _ => return None,
        };
        let mut system_choices = current.system_choices.clone();
        for (choice, selected_index) in system_choices.iter_mut().zip(&indices[5..]) {
            choice.selected = choice.options.get(*selected_index)?.0.clone();
        }

        Some(Self {
            audio_muted,
            master_volume_percent,
            latency_ms,
            sample_rate,
            vsync,
            system_choices,
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

/// JNI bridge state: bundled into one struct to avoid multiple statics.
struct SettingsBridge {
    result: Option<SettingsDialogResult>,
    waker: Option<AndroidAppWaker>,
    in_flight: bool,
    cached: CachedSettingsData,
}

impl SettingsBridge {
    const fn empty() -> Self {
        Self {
            result: None,
            waker: None,
            in_flight: false,
            cached: CachedSettingsData::empty(),
        }
    }
}

static SETTINGS: Mutex<SettingsBridge> = Mutex::new(SettingsBridge::empty());

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
    let keys = current.dialog_keys();
    let labels = current.dialog_labels();
    let choices = current.dialog_choices();
    let current_indices = current.current_indices();
    SETTINGS.lock().expect("settings mutex poisoned").cached = CachedSettingsData {
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
    let mut guard = SETTINGS.lock().expect("settings mutex poisoned");
    if guard.in_flight {
        return Ok(false);
    }
    guard.in_flight = true;
    let keys = guard.cached.keys.clone();
    let labels = guard.cached.labels.clone();
    let choices = guard.cached.choices.clone();
    let current_indices = guard.cached.current_indices.clone();
    drop(guard);

    if let Err(error) =
        show_settings_with_env(env, activity, &keys, &labels, &choices, &current_indices)
    {
        SETTINGS.lock().expect("settings mutex poisoned").in_flight = false;
        return Err(error);
    }
    Ok(true)
}

pub(crate) fn bind_app(app: &AndroidApp) {
    let mut guard = SETTINGS.lock().expect("settings mutex poisoned");
    guard.waker = Some(app.create_waker());
    guard.result = None;
    guard.in_flight = false;
}

pub(crate) fn reset() {
    let mut guard = SETTINGS.lock().expect("settings mutex poisoned");
    guard.result = None;
    guard.in_flight = false;
}

pub(crate) fn take_result() -> Option<SettingsDialogResult> {
    SETTINGS
        .lock()
        .expect("settings mutex poisoned")
        .result
        .take()
}

/// Request that the Android side show the settings dialog.
///
/// Returns `Ok(false)` when a dialog is already in flight (idempotent guard).
pub(crate) fn request_show_settings_dialog(
    app: &AndroidApp,
    current: &AndroidSettings,
) -> Result<bool, String> {
    {
        let mut guard = SETTINGS.lock().expect("settings mutex poisoned");
        if guard.in_flight {
            return Ok(false);
        }
        guard.in_flight = true;
    }

    let keys = current.dialog_keys();
    let labels = current.dialog_labels();
    let choices = current.dialog_choices();
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
            SETTINGS.lock().expect("settings mutex poisoned").in_flight = false;
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
    let mut guard = SETTINGS.lock().expect("settings mutex poisoned");
    guard.result = Some(result);
    guard.in_flight = false;
    drop(guard);
    wake_main_thread();
}

fn wake_main_thread() {
    if let Some(waker) = SETTINGS
        .lock()
        .expect("settings mutex poisoned")
        .waker
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
    use std::sync::Arc;

    use nerust_core_traits::factory::CoreFactory;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_settings::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };
    use nerust_nes_factory::NesFactory;
    use nerust_nes_settings::{NesSettings, NesVideoFilter};

    use super::*;

    fn default_snapshot() -> SettingsSnapshot {
        let mut shared = DesktopSharedSettings::default();
        shared.systems.insert(
            NesFactory.system_id(),
            Box::new(NesSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
        );
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn registry() -> SystemRegistry {
        SystemRegistry::new(vec![Arc::new(NesFactory)])
    }

    fn android_settings(snapshot: &SettingsSnapshot, registry: &SystemRegistry) -> AndroidSettings {
        AndroidSettings::from_snapshot(snapshot, registry)
    }

    fn set_system_choice(settings: &mut AndroidSettings, field_id: &str, choice_id: &str) {
        let choice = settings
            .system_choices
            .iter_mut()
            .find(|choice| choice.field_id.as_str() == field_id)
            .expect("system field should exist");
        choice.selected = SystemSettingsChoiceId(choice_id.to_string().into());
    }

    #[test]
    fn round_trips_default_snapshot() {
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let mut out = default_snapshot();
        android.apply_to_snapshot(&mut out, &registry).unwrap();
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
        let nes = snapshot
            .shared
            .systems
            .get_mut(NesFactory.system_id().as_ref())
            .map(|s| s.downcast_mut::<NesSettings>().unwrap())
            .unwrap();
        nes.video.filter = NesVideoFilter::NtscSVideo;

        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let filter = android
            .system_choices
            .iter()
            .find(|choice| choice.field_id.as_str() == "video.filter")
            .unwrap();
        assert_eq!(filter.selected.as_str(), "ntsc_svideo");
    }

    #[test]
    fn apply_to_snapshot_writes_all_fields() {
        let registry = registry();
        let mut snapshot = default_snapshot();
        let mut android = android_settings(&snapshot, &registry);
        android.audio_muted = true;
        android.master_volume_percent = 50;
        android.latency_ms = 75;
        android.sample_rate = 44_100;
        android.vsync = false;
        set_system_choice(&mut android, "video.filter", "ntsc_rgb");
        android.apply_to_snapshot(&mut snapshot, &registry).unwrap();

        assert!(snapshot.local.audio.muted);
        assert_eq!(snapshot.local.audio.master_volume_percent, 50);
        assert_eq!(snapshot.local.audio.latency_ms, 75);
        assert_eq!(snapshot.local.audio.sample_rate, 44_100);
        assert!(!snapshot.local.video.presentation.vsync);
        let nes = snapshot
            .shared
            .systems
            .get(NesFactory.system_id().as_ref())
            .map(|s| s.downcast_ref::<NesSettings>().unwrap())
            .unwrap();
        assert_eq!(nes.video.filter, NesVideoFilter::NtscRgb);
    }

    #[test]
    fn current_indices_matches_defaults() {
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let indices = android.current_indices();
        // Default: not muted → 0; volume 100% → index 100; latency 50 ms → index 40;
        // sample rate 48000 → index 1; vsync on → 1; NtscComposite → index 1
        assert_eq!(indices, vec!["0", "100", "40", "1", "1", "1", "0"]);
    }

    #[test]
    fn from_choice_indices_round_trips() {
        let registry = registry();
        let mut snapshot = default_snapshot();
        let mut original = android_settings(&snapshot, &registry);
        original.audio_muted = true;
        original.master_volume_percent = 25;
        original.latency_ms = 100;
        original.sample_rate = 44_100;
        original.vsync = false;
        set_system_choice(&mut original, "video.filter", "ntsc_svideo");
        original
            .apply_to_snapshot(&mut snapshot, &registry)
            .unwrap();
        let recovered = android_settings(&snapshot, &registry);
        let indices_str = recovered.current_indices().join(",");

        let parsed = AndroidSettings::from_choice_indices(&indices_str, &recovered).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_choice_indices_round_trips_non_default_audio_values() {
        let registry = registry();
        let mut original = android_settings(&default_snapshot(), &registry);
        original.audio_muted = false;
        original.master_volume_percent = 83;
        original.latency_ms = 37;
        original.sample_rate = 44_100;
        original.vsync = true;
        set_system_choice(&mut original, "video.filter", "none");

        let indices_str = original.current_indices().join(",");
        let parsed = AndroidSettings::from_choice_indices(&indices_str, &original).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_choice_indices_rejects_out_of_range() {
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        assert!(AndroidSettings::from_choice_indices("0,101,1,1,1,1,0", &current).is_none());
        assert!(AndroidSettings::from_choice_indices("0,4,191,1,1,1,0", &current).is_none());
        assert!(AndroidSettings::from_choice_indices("2,4,1,1,1,1,0", &current).is_none());
        assert!(AndroidSettings::from_choice_indices("0,4,1,1,2,1,0", &current).is_none());
    }

    #[test]
    fn dialog_choices_cover_full_android_audio_range() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let choices = android.dialog_choices();
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
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        assert!(AndroidSettings::from_choice_indices("0,4,1,1,1", &current).is_none());
        assert!(AndroidSettings::from_choice_indices("0,4,1,1,1,1,0,0", &current).is_none());
    }

    #[test]
    fn dialog_arrays_are_consistent_length() {
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let n = android.dialog_keys().len();
        assert_eq!(android.dialog_labels().len(), n);
        assert_eq!(android.dialog_choices().len(), n);
        assert_eq!(android.current_indices().len(), n);
    }
}
