use std::{collections::BTreeMap, sync::Mutex};

/// Android-relevant settings subset and JNI dialog bridge.
///
/// Only the fields that make sense on a mobile/touch device are exposed.
/// All persistence and validation remain on the Rust side; Kotlin merely
/// presents the choices and returns the user's selections.
use jni::objects::{JObject, JString, JValue};
use jni::{JavaVM, jni_sig, jni_str, refs::Global, sys::jobject};
use nerust_core_traits::{
    factory::{
        FactoryError,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind},
    },
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::{local::TouchOverlayVisibility, shared::StoragePolicy};
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::factory::{apply_settings_choice, resolve_label, settings_view};
use winit::platform::android::activity::AndroidApp;

use super::{bridge, messages::SettingsDialogResult};

// ---------------------------------------------------------------------------
// Choice constants
// ---------------------------------------------------------------------------

const VOLUME_MIN: u8 = 0;
const VOLUME_MAX: u8 = 100;
const LATENCY_MIN: u16 = 10;
const LATENCY_MAX: u16 = 200;
const SAMPLE_RATE_CHOICES: &[u32] = &[44_100, 48_000];
const OVERLAY_SCALE_MIN: u8 = 50;
const OVERLAY_SCALE_MAX: u8 = 150;
const OVERLAY_OFFSET_MIN: i8 = -30;
const OVERLAY_OFFSET_MAX: i8 = 30;
const SETTINGS_SCHEMA_VERSION: u32 = 1;

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
    pub storage_policy: StoragePolicy,
    pub overlay_visibility: TouchOverlayVisibility,
    pub overlay_opacity_percent: u8,
    pub overlay_scale_percent: u8,
    pub overlay_vertical_offset_percent: i8,
    pub overlay_haptics: bool,
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
    pub(crate) fn prioritize_system(&mut self, active: Option<&dyn SystemId>) {
        let Some(active) = active else {
            return;
        };
        self.system_choices
            .sort_by_key(|choice| choice.system_id.as_ref() != active);
    }

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
            storage_policy: snapshot.shared.persistence.storage_policy,
            overlay_visibility: snapshot.local.touch_overlay.visibility,
            overlay_opacity_percent: snapshot.local.touch_overlay.opacity_percent,
            overlay_scale_percent: snapshot.local.touch_overlay.scale_percent,
            overlay_vertical_offset_percent: snapshot.local.touch_overlay.vertical_offset_percent,
            overlay_haptics: snapshot.local.touch_overlay.haptics,
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
        snapshot.shared.persistence.storage_policy = self.storage_policy;
        snapshot.local.touch_overlay.visibility = self.overlay_visibility;
        snapshot.local.touch_overlay.opacity_percent = self.overlay_opacity_percent;
        snapshot.local.touch_overlay.scale_percent = self.overlay_scale_percent;
        snapshot.local.touch_overlay.vertical_offset_percent = self.overlay_vertical_offset_percent;
        snapshot.local.touch_overlay.haptics = self.overlay_haptics;

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
            "storage.policy".to_string(),
            "controls.overlay.visibility".to_string(),
            "controls.overlay.opacity".to_string(),
            "controls.overlay.scale".to_string(),
            "controls.overlay.vertical_offset".to_string(),
            "controls.overlay.haptics".to_string(),
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
            "Save Location".to_string(),
            "Touch Overlay".to_string(),
            "Overlay Opacity".to_string(),
            "Overlay Scale".to_string(),
            "Overlay Vertical Position".to_string(),
            "Overlay Haptics".to_string(),
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
            "Next to ROM\tApp Storage\tCustom Directory".to_string(),
            "Always\tAuto\tHidden".to_string(),
            join_tab_labels((0..=100).map(|value| format!("{value}%"))),
            join_tab_labels(
                (OVERLAY_SCALE_MIN..=OVERLAY_SCALE_MAX).map(|value| format!("{value}%")),
            ),
            join_tab_labels(
                (OVERLAY_OFFSET_MIN..=OVERLAY_OFFSET_MAX).map(|value| format!("{value}%")),
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
            match self.storage_policy {
                StoragePolicy::Sidecar => 0,
                StoragePolicy::AppSharedData => 1,
                StoragePolicy::CustomDirectory => 2,
            }
            .to_string(),
            match self.overlay_visibility {
                TouchOverlayVisibility::Always => 0,
                TouchOverlayVisibility::Auto => 1,
                TouchOverlayVisibility::Hidden => 2,
            }
            .to_string(),
            usize::from(self.overlay_opacity_percent.min(100)).to_string(),
            usize::from(
                self.overlay_scale_percent
                    .clamp(OVERLAY_SCALE_MIN, OVERLAY_SCALE_MAX)
                    - OVERLAY_SCALE_MIN,
            )
            .to_string(),
            i16::from(
                self.overlay_vertical_offset_percent
                    .clamp(OVERLAY_OFFSET_MIN, OVERLAY_OFFSET_MAX),
            )
            .checked_sub(i16::from(OVERLAY_OFFSET_MIN))
            .unwrap_or_default()
            .to_string(),
            (self.overlay_haptics as usize).to_string(),
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
        let storage_policy = match indices[5] {
            0 => StoragePolicy::Sidecar,
            1 => StoragePolicy::AppSharedData,
            2 => StoragePolicy::CustomDirectory,
            _ => return None,
        };
        let overlay_visibility = match indices[6] {
            0 => TouchOverlayVisibility::Always,
            1 => TouchOverlayVisibility::Auto,
            2 => TouchOverlayVisibility::Hidden,
            _ => return None,
        };
        let overlay_opacity_percent = u8::try_from(indices[7])
            .ok()
            .filter(|value| *value <= 100)?;
        let overlay_scale_percent = u8::try_from(indices[8])
            .ok()
            .filter(|value| *value <= OVERLAY_SCALE_MAX - OVERLAY_SCALE_MIN)?
            + OVERLAY_SCALE_MIN;
        let overlay_vertical_offset_percent = i8::try_from(indices[9])
            .ok()
            .filter(|value| *value <= OVERLAY_OFFSET_MAX - OVERLAY_OFFSET_MIN)?
            + OVERLAY_OFFSET_MIN;
        let overlay_haptics = match indices[10] {
            0 => false,
            1 => true,
            _ => return None,
        };
        let mut system_choices = current.system_choices.clone();
        for (choice, selected_index) in system_choices.iter_mut().zip(&indices[11..]) {
            choice.selected = choice.options.get(*selected_index)?.0.clone();
        }

        Some(Self {
            audio_muted,
            master_volume_percent,
            latency_ms,
            sample_rate,
            vsync,
            storage_policy,
            overlay_visibility,
            overlay_opacity_percent,
            overlay_scale_percent,
            overlay_vertical_offset_percent,
            overlay_haptics,
            system_choices,
        })
    }

    pub(crate) fn dialog_payload(&self, request_id: u64) -> String {
        let keys = self.dialog_keys();
        let labels = self.dialog_labels();
        let choices = self.dialog_choices();
        let indices = self.current_indices();
        let mut sections: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            let section = if key.starts_with("system.") {
                key.split('.').take(2).collect::<Vec<_>>().join(".")
            } else if key.starts_with("controls.") {
                "controls".to_string()
            } else if key.starts_with("storage.") {
                "storage".to_string()
            } else if key == "vsync" {
                "video".to_string()
            } else {
                "audio".to_string()
            };
            let field = serde_json::json!({
                    "key": key,
                    "label": labels[index],
                    "kind": "choice",
                    "value": indices[index].parse::<usize>().unwrap_or_default(),
                    "options": choices[index].split('\t').collect::<Vec<_>>(),
                    "enabled": true,
            });
            if let Some((_, fields)) = sections.iter_mut().find(|(id, _)| id == &section) {
                fields.push(field);
            } else {
                sections.push((section, vec![field]));
            }
        }
        let sections: Vec<_> = sections
            .into_iter()
            .map(|(id, fields)| serde_json::json!({ "id": id, "fields": fields }))
            .collect();
        serde_json::json!({
            "schemaVersion": SETTINGS_SCHEMA_VERSION,
            "requestId": request_id,
            "sections": sections,
        })
        .to_string()
    }

    pub(crate) fn from_keyed_indices(
        values: &BTreeMap<String, usize>,
        current: &Self,
    ) -> Option<Self> {
        let keys = current.dialog_keys();
        if values.len() != keys.len() || keys.iter().any(|key| !values.contains_key(key)) {
            return None;
        }
        let raw = keys
            .iter()
            .map(|key| values.get(key).map(usize::to_string))
            .collect::<Option<Vec<_>>>()?
            .join(",");
        Self::from_choice_indices(&raw, current)
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResultPayload {
    schema_version: u32,
    request_id: u64,
    #[serde(default)]
    dismissed: bool,
    #[serde(default)]
    values: BTreeMap<String, usize>,
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

struct SettingsBridgeState {
    result: Option<SettingsDialogResult>,
    cache: Option<AndroidSettings>,
    next_request_id: u64,
    pending_request_id: Option<u64>,
}

static SETTINGS_STATE: Mutex<SettingsBridgeState> = Mutex::new(SettingsBridgeState {
    result: None,
    cache: None,
    next_request_id: 1,
    pending_request_id: None,
});

fn with_state<T>(operation: impl FnOnce(&mut SettingsBridgeState) -> T) -> T {
    operation(
        &mut SETTINGS_STATE
            .lock()
            .expect("settings bridge mutex poisoned"),
    )
}

/// Update cached settings so `show_settings_dialog_sync` can present current data.
pub(crate) fn update_cached_settings(current: &AndroidSettings) {
    with_state(|state| state.cache = Some(current.clone()));
}

fn begin_request(current: &AndroidSettings) -> Option<(u64, String)> {
    with_state(|state| {
        if state.pending_request_id.is_some() {
            return None;
        }
        let request_id = state.next_request_id;
        state.next_request_id = state.next_request_id.wrapping_add(1).max(1);
        state.pending_request_id = Some(request_id);
        Some((request_id, current.dialog_payload(request_id)))
    })
}

/// Show the settings dialog synchronously from a JNI callback running on the
/// Java main thread.  Returns `Ok(false)` if a dialog is already in flight.
pub(crate) fn show_settings_dialog_sync(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
) -> Result<bool, String> {
    let current = with_state(|state| state.cache.clone())
        .ok_or_else(|| "Android settings cache is empty".to_string())?;
    let Some((request_id, payload)) = begin_request(&current) else {
        return Ok(false);
    };
    if let Err(error) = show_settings_with_env(env, activity, &payload) {
        with_state(|state| {
            if state.pending_request_id == Some(request_id) {
                state.pending_request_id = None;
            }
        });
        return Err(error);
    }
    Ok(true)
}

pub(crate) fn take_result() -> Option<SettingsDialogResult> {
    with_state(|state| state.result.take())
}

pub(crate) fn reset_transient() {
    with_state(|state| {
        state.result = None;
        state.pending_request_id = None;
    });
}

/// Request that the Android side show the settings dialog.
///
/// Returns `Ok(false)` when a dialog is already in flight (idempotent guard).
pub(crate) fn request_show_settings_dialog(
    app: &AndroidApp,
    current: &AndroidSettings,
) -> Result<bool, String> {
    let Some((request_id, payload)) = begin_request(current) else {
        return Ok(false);
    };

    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = show_settings_on_java_main_thread(&callback_app, &payload) {
            log::error!("{error}");
            with_state(|state| {
                if state.pending_request_id == Some(request_id) {
                    state.pending_request_id = None;
                }
            });
            bridge::wake();
        }
    }));
    Ok(true)
}

fn show_settings_on_java_main_thread(app: &AndroidApp, payload: &str) -> Result<(), String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    vm.attach_current_thread(|env| {
        let activity_raw = app.activity_as_ptr() as jobject;
        let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
        show_settings_with_env_inner(env, activity.as_ref(), payload)
    })
    .map_err(|error| format!("failed to show Android settings dialog: {error:?}"))
}

fn show_settings_with_env(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    payload: &str,
) -> Result<(), String> {
    env.with_local_frame(8, |env| {
        show_settings_with_env_inner(env, activity, payload)
    })
    .map_err(|error| format!("failed to show Android settings dialog: {error:?}"))
}

fn show_settings_with_env_inner(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    payload: &str,
) -> Result<(), jni::errors::Error> {
    let payload = env.new_string(payload)?;

    env.call_method(
        activity,
        jni_str!("showSettingsDialog"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(payload.as_ref())],
    )?;
    Ok(())
}

fn publish_result(request_id: u64, result: SettingsDialogResult) {
    let accepted = with_state(|state| {
        if state.pending_request_id != Some(request_id) {
            return false;
        }
        state.result = Some(result);
        state.pending_request_id = None;
        true
    });
    if !accepted {
        log::warn!("ignoring stale Android settings result for request {request_id}");
        return;
    }
    bridge::wake();
}

// ---------------------------------------------------------------------------
// JNI callback – invoked by `MainActivity.onSettingsDialogResult`
// ---------------------------------------------------------------------------
//
// * `result == null`  → dialog was dismissed
// * `result` is a versioned JSON object containing request ID and keyed values.

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onSettingsDialogResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    result: JString<'_>,
) {
    match env
        .with_env(|env| -> jni::errors::Result<Option<String>> {
            if result.is_null() {
                Ok(None)
            } else {
                Ok(Some(result.try_to_string(env)?))
            }
        })
        .into_outcome()
    {
        jni::Outcome::Ok(Some(raw)) => match serde_json::from_str::<SettingsResultPayload>(&raw) {
            Ok(payload) if payload.schema_version == SETTINGS_SCHEMA_VERSION => {
                let result = if payload.dismissed {
                    SettingsDialogResult::Dismissed
                } else {
                    SettingsDialogResult::Applied(payload.values)
                };
                publish_result(payload.request_id, result);
            }
            Ok(payload) => log::error!(
                "unsupported Android settings result schema {}",
                payload.schema_version
            ),
            Err(error) => log::error!("failed to parse Android settings result: {error}"),
        },
        jni::Outcome::Ok(None) => log::warn!("ignoring settings result without request ID"),
        jni::Outcome::Err(error) => {
            log::error!("failed to decode Android settings dialog result: {error:?}");
            with_state(|state| state.pending_request_id = None);
        }
        jni::Outcome::Panic(_) => {
            log::error!("Android settings dialog callback panicked");
            with_state(|state| state.pending_request_id = None);
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
    use nerust_gbc_factory::GbcFactory;
    use nerust_gbc_settings::GbcSettings;
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
        shared.systems.insert(
            GbcFactory.system_id(),
            Box::new(GbcSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
        );
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn registry() -> SystemRegistry {
        SystemRegistry::new(vec![Arc::new(NesFactory), Arc::new(GbcFactory)])
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
        assert_eq!(
            &indices[..11],
            ["0", "100", "40", "1", "1", "0", "1", "65", "50", "30", "1"]
        );
        assert_eq!(indices.len(), 15);
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

    #[test]
    fn dialog_payload_contains_schema_request_and_stable_keys() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let payload: serde_json::Value = serde_json::from_str(&android.dialog_payload(42)).unwrap();

        assert_eq!(payload["schemaVersion"], 1);
        assert_eq!(payload["requestId"], 42);
        assert_eq!(payload["sections"][0]["id"], "audio");
        assert_eq!(payload["sections"][0]["fields"][0]["key"], "audio_muted");
        let fields: Vec<_> = payload["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section["fields"].as_array().unwrap())
            .collect();
        let keys: Vec<_> = fields
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect();
        let volume = fields
            .iter()
            .find(|field| field["key"] == "master_volume")
            .unwrap();
        assert_eq!(volume["options"][0], "0%");
        assert_eq!(volume["options"][100], "100%");
        let storage = fields
            .iter()
            .find(|field| field["key"] == "storage.policy")
            .unwrap();
        assert_eq!(storage["options"][0], "Next to ROM");
        assert_eq!(storage["options"][1], "App Storage");
        assert!(keys.iter().any(|key| key.starts_with("system.gbc.")));
    }

    #[test]
    fn keyed_indices_do_not_depend_on_map_order() {
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        let mut values = BTreeMap::new();
        for (key, value) in current
            .dialog_keys()
            .into_iter()
            .zip(current.current_indices())
            .rev()
        {
            values.insert(key, value.parse().unwrap());
        }

        assert_eq!(
            AndroidSettings::from_keyed_indices(&values, &current),
            Some(current)
        );
    }
}
