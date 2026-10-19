use std::{collections::BTreeMap, sync::Mutex};

/// Android-relevant settings subset and JNI dialog bridge.
///
/// Only the fields that make sense on a mobile/touch device are exposed.
/// All persistence and validation remain on the Rust side; Kotlin merely
/// presents the choices and returns the user's selections.
use jni::objects::{JObject, JString, JValue};
use jni::{JavaVM, jni_sig, jni_str, sys::jobject};
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
use nerust_settings_core::model::{
    local::{
        NotificationAreaMode, NotificationPosition, RumbleTarget, ScreenOrientation,
        TouchOverlayVisibility,
    },
    shared::StoragePolicy,
};
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
    pub screen_orientation: ScreenOrientation,
    pub storage_policy: StoragePolicy,
    pub overlay_visibility: TouchOverlayVisibility,
    pub overlay_opacity_percent: u8,
    pub overlay_scale_percent: u8,
    pub overlay_vertical_offset_percent: i8,
    pub overlay_haptics: bool,
    pub motion_enabled: bool,
    pub rumble_enabled: bool,
    pub rumble_strength_percent: u8,
    pub rumble_target: RumbleTarget,
    pub notifications_info_enabled: bool,
    pub notifications_warning_enabled: bool,
    pub notifications_error_enabled: bool,
    pub notifications_position: NotificationPosition,
    pub notifications_area_mode: NotificationAreaMode,
    pub status_enabled: bool,
    pub status_position: NotificationPosition,
    pub status_area_mode: NotificationAreaMode,
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

/// One settings-dialog row: stable key, label, offered choices and
/// current choice index travel together. Every dialog array and codec
/// derives from [`AndroidSettings::setting_defs`], so a row can never
/// drift apart from its own choices again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingDef {
    pub key: String,
    pub label: String,
    pub choices: Vec<String>,
    pub current: usize,
    /// Edge class per choice ("top"/"bottom") for position pickers.
    /// The dialog hides same-edge followers from these labels, so the
    /// index→edge mapping lives in Rust alone. Empty for other rows.
    pub edges: Vec<String>,
}

/// Canonical corner order shared by labels, indices, and edges.
/// Every position picker follows this sequence; nothing else may
/// assume an index→corner mapping.
const CORNERS: [NotificationPosition; 4] = [
    NotificationPosition::BottomLeft,
    NotificationPosition::BottomRight,
    NotificationPosition::TopLeft,
    NotificationPosition::TopRight,
];

const CORNER_LABELS: [&str; 4] = ["Bottom Left", "Bottom Right", "Top Left", "Top Right"];

fn corner_edges() -> Vec<String> {
    CORNERS
        .into_iter()
        .map(|pos| {
            if pos.is_top() {
                "top".to_string()
            } else {
                "bottom".to_string()
            }
        })
        .collect()
}

fn corner_index(position: NotificationPosition) -> usize {
    CORNERS
        .iter()
        .position(|&corner| corner == position)
        .unwrap_or(0)
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
            screen_orientation: snapshot.local.screen_orientation,
            storage_policy: snapshot.shared.persistence.storage_policy,
            overlay_visibility: snapshot.local.touch_overlay.visibility,
            overlay_opacity_percent: snapshot.local.touch_overlay.opacity_percent,
            overlay_scale_percent: snapshot.local.touch_overlay.scale_percent,
            overlay_vertical_offset_percent: snapshot.local.touch_overlay.vertical_offset_percent,
            overlay_haptics: snapshot.local.touch_overlay.haptics,
            motion_enabled: snapshot.local.cartridge_peripherals.motion_enabled,
            rumble_enabled: snapshot.local.cartridge_peripherals.rumble_enabled,
            rumble_strength_percent: snapshot.local.cartridge_peripherals.rumble_strength_percent,
            rumble_target: snapshot.local.cartridge_peripherals.rumble_target,
            notifications_info_enabled: snapshot.local.notifications.info_enabled,
            notifications_warning_enabled: snapshot.local.notifications.warning_enabled,
            notifications_error_enabled: snapshot.local.notifications.error_enabled,
            notifications_position: snapshot.local.notifications.position,
            notifications_area_mode: snapshot.local.notifications.area_mode,
            status_enabled: snapshot.local.notifications.status_enabled,
            status_position: snapshot.local.notifications.status_position,
            status_area_mode: snapshot.local.notifications.status_area_mode,
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
        snapshot.local.screen_orientation = self.screen_orientation;
        snapshot.shared.persistence.storage_policy = self.storage_policy;
        snapshot.local.touch_overlay.visibility = self.overlay_visibility;
        snapshot.local.touch_overlay.opacity_percent = self.overlay_opacity_percent;
        snapshot.local.touch_overlay.scale_percent = self.overlay_scale_percent;
        snapshot.local.touch_overlay.vertical_offset_percent = self.overlay_vertical_offset_percent;
        snapshot.local.touch_overlay.haptics = self.overlay_haptics;
        snapshot.local.cartridge_peripherals.motion_enabled = self.motion_enabled;
        snapshot.local.cartridge_peripherals.rumble_enabled = self.rumble_enabled;
        snapshot.local.cartridge_peripherals.rumble_strength_percent = self.rumble_strength_percent;
        snapshot.local.cartridge_peripherals.rumble_target = self.rumble_target;
        snapshot.local.notifications.info_enabled = self.notifications_info_enabled;
        snapshot.local.notifications.warning_enabled = self.notifications_warning_enabled;
        snapshot.local.notifications.error_enabled = self.notifications_error_enabled;
        snapshot.local.notifications.position = self.notifications_position;
        snapshot.local.notifications.area_mode = self.notifications_area_mode;
        snapshot.local.notifications.status_enabled = self.status_enabled;
        snapshot.local.notifications.status_position = self.status_position;
        snapshot.local.notifications.status_area_mode = self.status_area_mode;

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

    /// One settings row: key, label, choices, and current index in one
    /// struct. Sole source of truth; codecs index this same list, so
    /// rows can never drift from their choices.
    pub(crate) fn setting_defs(&self) -> Vec<SettingDef> {
        let off_on = || vec!["Off".to_string(), "On".to_string()];
        let percent = |range: std::ops::RangeInclusive<u8>| {
            range.map(|value| format!("{value}%")).collect::<Vec<_>>()
        };
        let mut defs = vec![
            SettingDef {
                key: "audio_muted".to_string(),
                label: "Mute".to_string(),
                choices: off_on(),
                current: usize::from(self.audio_muted),
                edges: Vec::new(),
            },
            SettingDef {
                key: "master_volume".to_string(),
                label: "Volume".to_string(),
                choices: percent(VOLUME_MIN..=VOLUME_MAX),
                current: usize::from(self.master_volume_percent.min(VOLUME_MAX)),
                edges: Vec::new(),
            },
            SettingDef {
                key: "latency_ms".to_string(),
                label: "Audio Latency (ms)".to_string(),
                choices: (LATENCY_MIN..=LATENCY_MAX)
                    .map(|value| format!("{value} ms"))
                    .collect(),
                current: usize::from(self.latency_ms.clamp(LATENCY_MIN, LATENCY_MAX) - LATENCY_MIN),
                edges: Vec::new(),
            },
            SettingDef {
                key: "sample_rate".to_string(),
                label: "Sample Rate (Hz)".to_string(),
                choices: SAMPLE_RATE_CHOICES
                    .iter()
                    .map(|value| format!("{value} Hz"))
                    .collect(),
                // Default: highest rate.
                current: SAMPLE_RATE_CHOICES
                    .iter()
                    .position(|&v| v == self.sample_rate)
                    .unwrap_or(SAMPLE_RATE_CHOICES.len().saturating_sub(1)),
                edges: Vec::new(),
            },
            SettingDef {
                key: "vsync".to_string(),
                label: "VSync".to_string(),
                choices: off_on(),
                current: usize::from(self.vsync),
                edges: Vec::new(),
            },
            SettingDef {
                key: "screen.orientation".to_string(),
                label: "Screen Orientation".to_string(),
                choices: ["Auto Rotate", "Portrait", "Landscape"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                current: match self.screen_orientation {
                    ScreenOrientation::Auto => 0,
                    ScreenOrientation::Portrait => 1,
                    ScreenOrientation::Landscape => 2,
                },
                edges: Vec::new(),
            },
            SettingDef {
                key: "storage.policy".to_string(),
                label: "Save Location".to_string(),
                choices: ["Next to ROM", "App Storage", "Custom Directory"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                current: match self.storage_policy {
                    StoragePolicy::Sidecar => 0,
                    StoragePolicy::AppSharedData => 1,
                    StoragePolicy::CustomDirectory => 2,
                },
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.overlay.visibility".to_string(),
                label: "Touch Overlay".to_string(),
                choices: ["Always", "Auto", "Hidden"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                current: match self.overlay_visibility {
                    TouchOverlayVisibility::Always => 0,
                    TouchOverlayVisibility::Auto => 1,
                    TouchOverlayVisibility::Hidden => 2,
                },
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.overlay.opacity".to_string(),
                label: "Overlay Opacity".to_string(),
                choices: percent(0..=100),
                current: usize::from(self.overlay_opacity_percent.min(100)),
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.overlay.scale".to_string(),
                label: "Overlay Scale".to_string(),
                choices: percent(OVERLAY_SCALE_MIN..=OVERLAY_SCALE_MAX),
                current: usize::from(
                    self.overlay_scale_percent
                        .clamp(OVERLAY_SCALE_MIN, OVERLAY_SCALE_MAX)
                        - OVERLAY_SCALE_MIN,
                ),
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.overlay.vertical_offset".to_string(),
                label: "Overlay Vertical Position".to_string(),
                choices: (OVERLAY_OFFSET_MIN..=OVERLAY_OFFSET_MAX)
                    .map(|value| format!("{value}%"))
                    .collect(),
                current: i16::from(
                    self.overlay_vertical_offset_percent
                        .clamp(OVERLAY_OFFSET_MIN, OVERLAY_OFFSET_MAX),
                )
                .checked_sub(i16::from(OVERLAY_OFFSET_MIN))
                .unwrap_or_default() as usize,
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.overlay.haptics".to_string(),
                label: "Overlay Haptics".to_string(),
                choices: off_on(),
                current: usize::from(self.overlay_haptics),
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.cartridge.motion".to_string(),
                label: "Cartridge Motion Sensor".to_string(),
                choices: off_on(),
                current: usize::from(self.motion_enabled),
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.cartridge.rumble".to_string(),
                label: "Cartridge Rumble".to_string(),
                choices: off_on(),
                current: usize::from(self.rumble_enabled),
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.cartridge.rumble_strength".to_string(),
                label: "Rumble Strength".to_string(),
                choices: percent(0..=100),
                current: usize::from(self.rumble_strength_percent.min(100)),
                edges: Vec::new(),
            },
            SettingDef {
                key: "controls.cartridge.rumble_target".to_string(),
                label: "Rumble Target".to_string(),
                choices: ["Auto", "Handset", "Controller"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                current: match self.rumble_target {
                    RumbleTarget::Auto => 0,
                    RumbleTarget::Handset => 1,
                    RumbleTarget::Controller => 2,
                },
                edges: Vec::new(),
            },
            SettingDef {
                key: "notifications.info".to_string(),
                label: "Info Notifications".to_string(),
                choices: off_on(),
                current: usize::from(self.notifications_info_enabled),
                edges: Vec::new(),
            },
            SettingDef {
                key: "notifications.warning".to_string(),
                label: "Warning Notifications".to_string(),
                choices: off_on(),
                current: usize::from(self.notifications_warning_enabled),
                edges: Vec::new(),
            },
            SettingDef {
                key: "notifications.error".to_string(),
                label: "Error Notifications".to_string(),
                choices: off_on(),
                current: usize::from(self.notifications_error_enabled),
                edges: Vec::new(),
            },
            SettingDef {
                key: "notifications.position".to_string(),
                label: "Notification Position".to_string(),
                choices: CORNER_LABELS.into_iter().map(str::to_string).collect(),
                current: corner_index(self.notifications_position),
                edges: corner_edges(),
            },
            SettingDef {
                key: "notifications.area_mode".to_string(),
                label: "Notification Area".to_string(),
                choices: ["Overlay", "Strip"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                current: match self.notifications_area_mode {
                    NotificationAreaMode::Overlay => 0,
                    NotificationAreaMode::Strip => 1,
                },
                edges: Vec::new(),
            },
            SettingDef {
                key: "status.enabled".to_string(),
                label: "Status Display".to_string(),
                choices: off_on(),
                current: usize::from(self.status_enabled),
                edges: Vec::new(),
            },
            SettingDef {
                key: "status.position".to_string(),
                label: "Status Position".to_string(),
                choices: CORNER_LABELS.into_iter().map(str::to_string).collect(),
                current: corner_index(self.status_position),
                edges: corner_edges(),
            },
            SettingDef {
                key: "status.area_mode".to_string(),
                label: "Status Area".to_string(),
                choices: ["Overlay", "Strip"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                current: match self.status_area_mode {
                    NotificationAreaMode::Overlay => 0,
                    NotificationAreaMode::Strip => 1,
                },
                edges: Vec::new(),
            },
        ];
        // Same-edge follow is handled live in the dialog UI (the
        // status row hides when it shares the notification edge);
        // the codec stays key-driven so order never matters.
        defs.extend(self.system_choices.iter().map(|choice| {
            SettingDef {
                key: format!("system.{}.{}", choice.system_id, choice.field_id.as_str()),
                label: choice.label.clone(),
                choices: choice
                    .options
                    .iter()
                    .map(|(_, label)| label.clone())
                    .collect(),
                current: choice
                    .options
                    .iter()
                    .position(|(id, _)| id == &choice.selected)
                    .unwrap_or_default(),
                edges: Vec::new(),
            }
        }));
        defs
    }

    /// Stable setting keys sent to Kotlin (also used for decoding the result).
    /// Build settings from a comma-separated list of choice indices.
    /// Index N is the choice for `setting_defs()[N]`; positions must
    /// come from the same defs list. `None` on malformed input.
    pub(crate) fn from_choice_indices(raw: &str, current: &Self) -> Option<Self> {
        let indices: Vec<usize> = raw
            .split(',')
            .map(|s| s.trim().parse::<usize>().ok())
            .collect::<Option<_>>()?;

        // Key-driven: field order never affects decoding, and a
        // row missing from the payload keeps its current value.
        let defs = current.setting_defs();
        if indices.len() != defs.len() {
            return None;
        }
        let by_key = |key: &str| -> Option<usize> {
            defs.iter()
                .position(|def| def.key == key)
                .and_then(|pos| indices.get(pos).copied())
        };
        let off_on = |key: &str| -> Option<bool> {
            match by_key(key)? {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            }
        };
        let position =
            |key: &str| -> Option<NotificationPosition> { CORNERS.get(by_key(key)?).copied() };
        let area_mode = |key: &str| -> Option<NotificationAreaMode> {
            match by_key(key)? {
                0 => Some(NotificationAreaMode::Overlay),
                1 => Some(NotificationAreaMode::Strip),
                _ => None,
            }
        };

        let audio_muted = off_on("audio_muted")?;
        let master_volume_percent = u8::try_from(by_key("master_volume")?)
            .ok()
            .filter(|value| *value <= VOLUME_MAX)?;
        let latency_ms = u16::try_from(by_key("latency_ms")?)
            .ok()
            .filter(|value| *value <= LATENCY_MAX - LATENCY_MIN)
            .map(|value| value + LATENCY_MIN)?;
        let sample_rate = *SAMPLE_RATE_CHOICES.get(by_key("sample_rate")?)?;
        let vsync = off_on("vsync")?;
        let screen_orientation = match by_key("screen.orientation")? {
            0 => ScreenOrientation::Auto,
            1 => ScreenOrientation::Portrait,
            2 => ScreenOrientation::Landscape,
            _ => return None,
        };
        let storage_policy = match by_key("storage.policy")? {
            0 => StoragePolicy::Sidecar,
            1 => StoragePolicy::AppSharedData,
            2 => StoragePolicy::CustomDirectory,
            _ => return None,
        };
        let overlay_visibility = match by_key("controls.overlay.visibility")? {
            0 => TouchOverlayVisibility::Always,
            1 => TouchOverlayVisibility::Auto,
            2 => TouchOverlayVisibility::Hidden,
            _ => return None,
        };
        let overlay_opacity_percent = u8::try_from(by_key("controls.overlay.opacity")?)
            .ok()
            .filter(|value| *value <= 100)?;
        let overlay_scale_percent = u8::try_from(by_key("controls.overlay.scale")?)
            .ok()
            .filter(|value| *value <= OVERLAY_SCALE_MAX - OVERLAY_SCALE_MIN)?
            + OVERLAY_SCALE_MIN;
        let overlay_vertical_offset_percent =
            i8::try_from(by_key("controls.overlay.vertical_offset")?)
                .ok()
                .filter(|value| *value <= OVERLAY_OFFSET_MAX - OVERLAY_OFFSET_MIN)?
                + OVERLAY_OFFSET_MIN;
        let overlay_haptics = off_on("controls.overlay.haptics")?;
        let motion_enabled = off_on("controls.cartridge.motion")?;
        let rumble_enabled = off_on("controls.cartridge.rumble")?;
        let rumble_strength_percent = u8::try_from(by_key("controls.cartridge.rumble_strength")?)
            .ok()
            .filter(|value| *value <= 100)?;
        let rumble_target = match by_key("controls.cartridge.rumble_target")? {
            0 => RumbleTarget::Auto,
            1 => RumbleTarget::Handset,
            2 => RumbleTarget::Controller,
            _ => return None,
        };
        let notifications_info_enabled = off_on("notifications.info")?;
        let notifications_warning_enabled = off_on("notifications.warning")?;
        let notifications_error_enabled = off_on("notifications.error")?;
        let notifications_position = position("notifications.position")?;
        let notifications_area_mode = area_mode("notifications.area_mode")?;
        let status_enabled = off_on("status.enabled")?;
        let status_position = position("status.position")?;
        // Absent when same-edge (follows the notification mode):
        // keep the current value instead of rejecting the payload.
        let status_area_mode = match defs.iter().any(|def| def.key == "status.area_mode") {
            true => area_mode("status.area_mode")?,
            false => current.status_area_mode,
        };
        let mut system_choices = current.system_choices.clone();
        for choice in system_choices.iter_mut() {
            let key = format!("system.{}.{}", choice.system_id, choice.field_id.as_str());
            let selected_index = by_key(&key)?;
            choice.selected = choice.options.get(selected_index)?.0.clone();
        }

        Some(Self {
            audio_muted,
            master_volume_percent,
            latency_ms,
            sample_rate,
            vsync,
            screen_orientation,
            storage_policy,
            overlay_visibility,
            overlay_opacity_percent,
            overlay_scale_percent,
            overlay_vertical_offset_percent,
            overlay_haptics,
            motion_enabled,
            rumble_enabled,
            rumble_strength_percent,
            rumble_target,
            notifications_info_enabled,
            notifications_warning_enabled,
            notifications_error_enabled,
            notifications_position,
            notifications_area_mode,
            status_enabled,
            status_position,
            status_area_mode,
            system_choices,
        })
    }

    pub(crate) fn dialog_payload(&self, request_id: u64) -> String {
        let mut sections: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for def in self.setting_defs() {
            let section = if def.key.starts_with("system.") {
                def.key.split('.').take(2).collect::<Vec<_>>().join(".")
            } else if def.key.starts_with("controls.") {
                "controls".to_string()
            } else if def.key.starts_with("storage.") {
                "storage".to_string()
            } else if def.key.starts_with("notifications.") || def.key.starts_with("status.") {
                "notifications".to_string()
            } else if def.key == "vsync" || def.key.starts_with("screen.") {
                "video".to_string()
            } else {
                "audio".to_string()
            };
            let mut field = serde_json::json!({
                    "key": def.key,
                    "label": def.label,
                    "kind": "choice",
                    "value": def.current,
                    "options": def.choices,
                    "enabled": true,
            });
            if !def.edges.is_empty() {
                field["edges"] = serde_json::json!(def.edges);
            }
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
        let defs = current.setting_defs();
        if values.len() != defs.len() || defs.iter().any(|def| !values.contains_key(&def.key)) {
            return None;
        }
        let raw = defs
            .iter()
            .map(|def| values.get(&def.key).map(usize::to_string))
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
    // A panicking JNI callback must not wedge the process on the
    // next lock: recover the state and keep serving events.
    let mut guard = SETTINGS_STATE.lock().unwrap_or_else(|poison| {
        log::error!("settings bridge mutex poisoned; recovering state");
        poison.into_inner()
    });
    operation(&mut guard)
}

/// Update cached settings so `show_settings_dialog_sync` can present current data.
pub(crate) fn update_cached_settings(current: &AndroidSettings) {
    with_state(|state| state.cache = Some(current.clone()));
}

pub(crate) fn apply_screen_orientation(app: &AndroidApp, orientation: ScreenOrientation) {
    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let result = unsafe { JavaVM::from_raw(callback_app.vm_as_ptr() as _) }
            .attach_current_thread(|env| {
                let activity_raw = callback_app.activity_as_ptr() as jobject;
                let activity = unsafe { JObject::from_raw(env, activity_raw) };
                let value = match orientation {
                    ScreenOrientation::Auto => 0,
                    ScreenOrientation::Portrait => 1,
                    ScreenOrientation::Landscape => 2,
                };
                env.call_method(
                    activity.as_ref(),
                    jni_str!("applyScreenOrientation"),
                    jni_sig!("(I)V"),
                    &[JValue::Int(value)],
                )?;
                Ok::<_, jni::errors::Error>(())
            });
        if let Err(error) = result {
            log::error!("failed to apply Android screen orientation: {error:?}");
        }
    }));
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

/// Whether a settings dialog is up (either show path: bridge pending id).
pub(crate) fn is_dialog_open() -> bool {
    with_state(|state| state.pending_request_id.is_some())
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
            // Never shown means never dismissed: synthesize the dismiss
            // so the frontend clears its open flag and flushes errors.
            publish_result(request_id, SettingsDialogResult::Dismissed);
        }
    }));
    Ok(true)
}

fn show_settings_on_java_main_thread(app: &AndroidApp, payload: &str) -> Result<(), String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    vm.attach_current_thread(|env| {
        let activity_raw = app.activity_as_ptr() as jobject;
        let activity = unsafe { JObject::from_raw(env, activity_raw) };
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

// JNI callback for the settings dialog result.
// Null means dismissed; otherwise versioned JSON with ID and values.

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
    use nerust_gba_factory::GbaFactory;
    use nerust_gba_settings::GbaSettings;
    use nerust_gbc_factory::GbcFactory;
    use nerust_gbc_settings::GbcSettings;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_nes_factory::NesFactory;
    use nerust_nes_settings::{NesSettings, NesVideoFilter};
    use nerust_settings_core::model::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };

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
        shared.systems.insert(
            GbaFactory.system_id(),
            Box::new(GbaSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
        );
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn registry() -> SystemRegistry {
        SystemRegistry::new(vec![
            Arc::new(NesFactory),
            Arc::new(GbcFactory),
            Arc::new(GbaFactory),
        ])
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

    /// Current choice indices in dialog order, via the single defs list.
    fn def_currents(android: &AndroidSettings) -> Vec<usize> {
        android
            .setting_defs()
            .into_iter()
            .map(|def| def.current)
            .collect()
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
        android.motion_enabled = false;
        android.rumble_enabled = false;
        android.rumble_strength_percent = 35;
        android.rumble_target = RumbleTarget::Controller;
        android.notifications_info_enabled = false;
        android.notifications_position = NotificationPosition::TopRight;
        android.notifications_area_mode = NotificationAreaMode::Strip;
        android.status_enabled = true;
        android.status_position = NotificationPosition::BottomRight;
        android.status_area_mode = NotificationAreaMode::Strip;
        set_system_choice(&mut android, "video.filter", "ntsc_rgb");
        android.apply_to_snapshot(&mut snapshot, &registry).unwrap();

        assert!(snapshot.local.audio.muted);
        assert_eq!(snapshot.local.audio.master_volume_percent, 50);
        assert_eq!(snapshot.local.audio.latency_ms, 75);
        assert_eq!(snapshot.local.audio.sample_rate, 44_100);
        assert!(!snapshot.local.video.presentation.vsync);
        assert!(!snapshot.local.cartridge_peripherals.motion_enabled);
        assert!(!snapshot.local.cartridge_peripherals.rumble_enabled);
        assert_eq!(
            snapshot.local.cartridge_peripherals.rumble_strength_percent,
            35
        );
        assert_eq!(
            snapshot.local.cartridge_peripherals.rumble_target,
            RumbleTarget::Controller
        );
        assert!(!snapshot.local.notifications.info_enabled);
        assert_eq!(
            snapshot.local.notifications.position,
            NotificationPosition::TopRight
        );
        assert_eq!(
            snapshot.local.notifications.area_mode,
            NotificationAreaMode::Strip
        );
        assert!(snapshot.local.notifications.status_enabled);
        assert_eq!(
            snapshot.local.notifications.status_position,
            NotificationPosition::BottomRight
        );
        assert_eq!(
            snapshot.local.notifications.status_area_mode,
            NotificationAreaMode::Strip
        );
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
        let indices = def_currents(&android);
        // Default: not muted → 0; volume 100% → index 100; latency 50 ms → index 40;
        // sample rate 48000 → index 1; vsync on → 1; NtscComposite → index 1.
        // Notifications: info/warning/error on → 1; position BottomLeft → 0;
        // areas Overlay → 0; status off → 0; status position TopLeft → 2.
        assert_eq!(
            &indices[..24],
            [
                0, 100, 40, 1, 1, 0, 0, 1, 65, 50, 30, 1, 1, 1, 100, 0, 1, 1, 1, 0, 0, 0, 2, 0
            ]
        );
        assert_eq!(indices.len(), 29);
    }

    #[test]
    fn dialog_payload_contains_gba_system_key() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let keys: Vec<String> = android
            .setting_defs()
            .into_iter()
            .map(|def| def.key)
            .collect();
        assert!(keys.iter().any(|key| key.starts_with("system.gba.")));
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
        original.screen_orientation = ScreenOrientation::Landscape;
        original.motion_enabled = false;
        original.rumble_enabled = false;
        original.rumble_strength_percent = 42;
        original.rumble_target = RumbleTarget::Handset;
        set_system_choice(&mut original, "video.filter", "ntsc_svideo");
        original
            .apply_to_snapshot(&mut snapshot, &registry)
            .unwrap();
        let recovered = android_settings(&snapshot, &registry);
        let indices_str = def_currents(&recovered)
            .into_iter()
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");

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
        original.screen_orientation = ScreenOrientation::Portrait;
        set_system_choice(&mut original, "video.filter", "none");

        let indices_str = def_currents(&original)
            .into_iter()
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
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
        let defs = android.setting_defs();
        let volume_choices = &defs[1].choices;
        let latency_choices = &defs[2].choices;
        let sample_rate_choices = &defs[3].choices;

        assert_eq!(volume_choices.first().map(String::as_str), Some("0%"));
        assert_eq!(volume_choices.last().map(String::as_str), Some("100%"));
        assert_eq!(volume_choices.len(), 101);

        assert_eq!(latency_choices.first().map(String::as_str), Some("10 ms"));
        assert_eq!(latency_choices.last().map(String::as_str), Some("200 ms"));
        assert_eq!(latency_choices.len(), 191);

        assert!(
            !sample_rate_choices.is_empty(),
            "sample rate choices should be non-empty"
        );
        for choice in sample_rate_choices {
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
    fn setting_defs_are_internally_consistent() {
        // Structural replacement for the old parallel-arrays length check:
        // every row must offer at least one choice, point at a valid one,
        // and carry a unique key.
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let defs = android.setting_defs();
        let mut keys = std::collections::BTreeSet::new();
        for def in &defs {
            assert!(keys.insert(def.key.clone()), "duplicate key {}", def.key);
            assert!(!def.label.is_empty(), "empty label for {}", def.key);
            assert!(!def.choices.is_empty(), "no choices for {}", def.key);
            assert!(
                def.current < def.choices.len(),
                "current {} out of range for {}",
                def.current,
                def.key
            );
        }
    }

    #[test]
    fn dialog_choices_align_with_keys() {
        // Regression test: the choice list attached to each key must be
        // that setting's own. A past misalignment showed percentages on
        // the visibility row and words on the opacity row, silently
        // rejecting every save outside 0..=2.
        use std::collections::BTreeMap;
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let owned: BTreeMap<String, Vec<String>> = android
            .setting_defs()
            .into_iter()
            .map(|def| (def.key, def.choices))
            .collect();
        let at = |key: &str| owned.get(key).unwrap().join("\t");
        assert_eq!(at("screen.orientation"), "Auto Rotate\tPortrait\tLandscape");
        assert_eq!(
            at("storage.policy"),
            "Next to ROM\tApp Storage\tCustom Directory"
        );
        assert_eq!(at("controls.overlay.visibility"), "Always\tAuto\tHidden");
        let opacity_str = at("controls.overlay.opacity");
        let opacity: Vec<_> = opacity_str.split('\t').collect();
        assert_eq!(opacity.len(), 101);
        assert_eq!(opacity.first(), Some(&"0%"));
        assert_eq!(opacity.last(), Some(&"100%"));
        let scale_str = at("controls.overlay.scale");
        let scale: Vec<_> = scale_str.split('\t').collect();
        assert_eq!(scale.len(), 101);
        assert_eq!(scale.first(), Some(&"50%"));
        assert_eq!(scale.last(), Some(&"150%"));
        let offset_str = at("controls.overlay.vertical_offset");
        let offset: Vec<_> = offset_str.split('\t').collect();
        assert_eq!(offset.len(), 61);
        assert_eq!(offset.first(), Some(&"-30%"));
        assert_eq!(offset.last(), Some(&"30%"));
        assert_eq!(at("controls.overlay.haptics"), "Off\tOn");
        assert_eq!(at("controls.cartridge.motion"), "Off\tOn");
        assert_eq!(at("controls.cartridge.rumble"), "Off\tOn");
        let strength_str = at("controls.cartridge.rumble_strength");
        let strength: Vec<_> = strength_str.split('\t').collect();
        assert_eq!(strength.len(), 101);
        assert_eq!(
            at("controls.cartridge.rumble_target"),
            "Auto\tHandset\tController"
        );
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
        let orientation = fields
            .iter()
            .find(|field| field["key"] == "screen.orientation")
            .unwrap();
        assert_eq!(orientation["options"][0], "Auto Rotate");
        assert_eq!(orientation["options"][1], "Portrait");
        assert_eq!(orientation["options"][2], "Landscape");
        assert!(
            payload["sections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|section| {
                    section["id"] == "video"
                        && section["fields"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|field| field["key"] == "screen.orientation")
                })
        );
        assert!(keys.iter().any(|key| key.starts_with("system.gbc.")));
    }

    #[test]
    fn dialog_payload_marks_position_edges() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let doc: serde_json::Value = serde_json::from_str(&android.dialog_payload(1)).unwrap();
        let fields: Vec<&serde_json::Value> = doc["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section["fields"].as_array().unwrap().iter())
            .collect();
        let position = fields
            .iter()
            .find(|field| field["key"] == "notifications.position")
            .unwrap();
        assert_eq!(
            position["edges"],
            serde_json::json!(["bottom", "bottom", "top", "top"])
        );
        let volume = fields
            .iter()
            .find(|field| field["key"] == "master_volume")
            .unwrap();
        assert!(volume.get("edges").is_none());
    }

    #[test]
    fn keyed_indices_do_not_depend_on_map_order() {
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        let mut values = BTreeMap::new();
        for def in current.setting_defs().into_iter().rev() {
            values.insert(def.key, def.current);
        }

        assert_eq!(
            AndroidSettings::from_keyed_indices(&values, &current),
            Some(current)
        );
    }

    #[test]
    fn same_edge_preserves_status_area() {
        let registry = registry();
        let mut android = android_settings(&default_snapshot(), &registry);
        android.status_position = NotificationPosition::BottomRight;
        android.status_area_mode = NotificationAreaMode::Strip;
        // Same edge as the default BottomLeft notification: the row
        // stays (the dialog UI hides it live), so the value follows
        // the notification mode at render time regardless.
        let defs = android.setting_defs();
        assert_eq!(defs.len(), 29);
        assert!(defs.iter().any(|def| def.key == "status.area_mode"));
        let raw = def_currents(&android)
            .into_iter()
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let back = AndroidSettings::from_choice_indices(&raw, &android).unwrap();
        assert_eq!(back.status_position, NotificationPosition::BottomRight);
        assert_eq!(back.status_area_mode, NotificationAreaMode::Strip);
    }
}
