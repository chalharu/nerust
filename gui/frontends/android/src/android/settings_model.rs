//! Settings model: Android-visible subset, defs, and fingerprint cache.
//! Pure data plus snapshot mapping; JNI and transport live elsewhere.

use std::sync::Arc;

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

// Choice constants.

pub(super) const VOLUME_MIN: u8 = 0;
pub(super) const VOLUME_MAX: u8 = 100;
pub(super) const LATENCY_MIN: u16 = 10;
pub(super) const LATENCY_MAX: u16 = 200;
pub(super) const SAMPLE_RATE_CHOICES: &[u32] = &[44_100, 48_000];
pub(super) const OVERLAY_SCALE_MIN: u8 = 50;
pub(super) const OVERLAY_SCALE_MAX: u8 = 150;
pub(super) const OVERLAY_OFFSET_MIN: i8 = -30;
pub(super) const OVERLAY_OFFSET_MAX: i8 = 30;
pub(super) const SETTINGS_SCHEMA_VERSION: u32 = 1;

// Data model.

/// The Android-relevant subset of the full settings snapshot.
/// Derived from [`SettingsSnapshot`] on the way in; applied back on the way out.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AndroidSettings {
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
    pub(super) system_choices: Vec<AndroidSystemChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AndroidSystemChoice {
    pub(super) system_id: Box<dyn SystemId>,
    pub(super) field_id: SystemSettingsFieldId,
    pub(super) label: String,
    pub(super) selected: SystemSettingsChoiceId,
    pub(super) options: Vec<(SystemSettingsChoiceId, String)>,
}

/// Ranged-int encoding: Kotlin expands `min..=max` with `unit`.
/// The 101-string tables are never built or shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RangeSpec {
    pub min: i64,
    pub max: i64,
    pub unit: &'static str,
}

/// One settings-dialog row: key, label, choices, and current index travel together.
/// All arrays and codecs derive from `setting_defs`; rows cannot drift from choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingDef {
    pub key: String,
    pub label: String,
    pub choices: Vec<String>,
    pub current: usize,
    /// `Some` for ranged ints: `choices` stays empty, Kotlin expands locally.
    pub range: Option<RangeSpec>,
    /// Same-edge rule resolved in Rust; Kotlin only filters on this flag.
    pub visible: bool,
}

/// Canonical corner order shared by labels and indices.
/// Every position picker follows this sequence; nothing else assumes the mapping.
pub(super) const CORNERS: [NotificationPosition; 4] = [
    NotificationPosition::BottomLeft,
    NotificationPosition::BottomRight,
    NotificationPosition::TopLeft,
    NotificationPosition::TopRight,
];

const CORNER_LABELS: [&str; 4] = ["Bottom Left", "Bottom Right", "Top Left", "Top Right"];

pub(super) fn corner_index(position: NotificationPosition) -> usize {
    CORNERS
        .iter()
        .position(|&corner| corner == position)
        .unwrap_or(0)
}

impl AndroidSettings {
    pub(super) fn prioritize_system(&mut self, active: Option<&dyn SystemId>) {
        let Some(active) = active else {
            return;
        };
        self.system_choices
            .sort_by_key(|choice| choice.system_id.as_ref() != active);
    }

    pub(super) fn from_snapshot(snapshot: &SettingsSnapshot, registry: &SystemRegistry) -> Self {
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
    pub(super) fn apply_to_snapshot(
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

    // Dialog encoding.

    /// One settings row: key, label, choices, and current index in one struct.
    pub(super) fn setting_defs(&self) -> Vec<SettingDef> {
        let off_on = || vec!["Off".to_string(), "On".to_string()];
        // Same-edge rule resolved here: the area row hides when both
        // positions share an edge; Kotlin only filters on `visible`.
        let status_area_visible =
            self.notifications_position.is_top() != self.status_position.is_top();
        let mut defs = vec![
            SettingDef {
                key: "audio_muted".to_string(),
                label: "Mute".to_string(),
                choices: off_on(),
                current: usize::from(self.audio_muted),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "master_volume".to_string(),
                label: "Volume".to_string(),
                choices: Vec::new(),
                current: usize::from(self.master_volume_percent.min(VOLUME_MAX)),
                range: Some(RangeSpec {
                    min: i64::from(VOLUME_MIN),
                    max: i64::from(VOLUME_MAX),
                    unit: "%",
                }),
                visible: true,
            },
            SettingDef {
                key: "latency_ms".to_string(),
                label: "Audio Latency (ms)".to_string(),
                choices: Vec::new(),
                current: usize::from(self.latency_ms.clamp(LATENCY_MIN, LATENCY_MAX) - LATENCY_MIN),
                range: Some(RangeSpec {
                    min: i64::from(LATENCY_MIN),
                    max: i64::from(LATENCY_MAX),
                    unit: " ms",
                }),
                visible: true,
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
                range: None,
                visible: true,
            },
            SettingDef {
                key: "vsync".to_string(),
                label: "VSync".to_string(),
                choices: off_on(),
                current: usize::from(self.vsync),
                range: None,
                visible: true,
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
                range: None,
                visible: true,
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
                range: None,
                visible: true,
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
                range: None,
                visible: true,
            },
            SettingDef {
                key: "controls.overlay.opacity".to_string(),
                label: "Overlay Opacity".to_string(),
                choices: Vec::new(),
                current: usize::from(self.overlay_opacity_percent.min(100)),
                range: Some(RangeSpec {
                    min: 0,
                    max: 100,
                    unit: "%",
                }),
                visible: true,
            },
            SettingDef {
                key: "controls.overlay.scale".to_string(),
                label: "Overlay Scale".to_string(),
                choices: Vec::new(),
                current: usize::from(
                    self.overlay_scale_percent
                        .clamp(OVERLAY_SCALE_MIN, OVERLAY_SCALE_MAX)
                        - OVERLAY_SCALE_MIN,
                ),
                range: Some(RangeSpec {
                    min: i64::from(OVERLAY_SCALE_MIN),
                    max: i64::from(OVERLAY_SCALE_MAX),
                    unit: "%",
                }),
                visible: true,
            },
            SettingDef {
                key: "controls.overlay.vertical_offset".to_string(),
                label: "Overlay Vertical Position".to_string(),
                choices: Vec::new(),
                current: i16::from(
                    self.overlay_vertical_offset_percent
                        .clamp(OVERLAY_OFFSET_MIN, OVERLAY_OFFSET_MAX),
                )
                .checked_sub(i16::from(OVERLAY_OFFSET_MIN))
                .unwrap_or_default() as usize,
                range: Some(RangeSpec {
                    min: i64::from(OVERLAY_OFFSET_MIN),
                    max: i64::from(OVERLAY_OFFSET_MAX),
                    unit: "%",
                }),
                visible: true,
            },
            SettingDef {
                key: "controls.overlay.haptics".to_string(),
                label: "Overlay Haptics".to_string(),
                choices: off_on(),
                current: usize::from(self.overlay_haptics),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "controls.cartridge.motion".to_string(),
                label: "Cartridge Motion Sensor".to_string(),
                choices: off_on(),
                current: usize::from(self.motion_enabled),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "controls.cartridge.rumble".to_string(),
                label: "Cartridge Rumble".to_string(),
                choices: off_on(),
                current: usize::from(self.rumble_enabled),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "controls.cartridge.rumble_strength".to_string(),
                label: "Rumble Strength".to_string(),
                choices: Vec::new(),
                current: usize::from(self.rumble_strength_percent.min(100)),
                range: Some(RangeSpec {
                    min: 0,
                    max: 100,
                    unit: "%",
                }),
                visible: true,
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
                range: None,
                visible: true,
            },
            SettingDef {
                key: "notifications.info".to_string(),
                label: "Info Notifications".to_string(),
                choices: off_on(),
                current: usize::from(self.notifications_info_enabled),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "notifications.warning".to_string(),
                label: "Warning Notifications".to_string(),
                choices: off_on(),
                current: usize::from(self.notifications_warning_enabled),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "notifications.error".to_string(),
                label: "Error Notifications".to_string(),
                choices: off_on(),
                current: usize::from(self.notifications_error_enabled),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "notifications.position".to_string(),
                label: "Notification Position".to_string(),
                choices: CORNER_LABELS.into_iter().map(str::to_string).collect(),
                current: corner_index(self.notifications_position),
                range: None,
                visible: true,
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
                range: None,
                visible: true,
            },
            SettingDef {
                key: "status.enabled".to_string(),
                label: "Status Display".to_string(),
                choices: off_on(),
                current: usize::from(self.status_enabled),
                range: None,
                visible: true,
            },
            SettingDef {
                key: "status.position".to_string(),
                label: "Status Position".to_string(),
                choices: CORNER_LABELS.into_iter().map(str::to_string).collect(),
                current: corner_index(self.status_position),
                range: None,
                visible: true,
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
                range: None,
                visible: status_area_visible,
            },
        ];
        // The codec stays key-driven so order never matters.
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
                range: None,
                visible: true,
            }
        }));
        defs
    }

    pub(super) fn cached_setting_defs_locked(
        entry: &mut Option<(u64, Arc<Vec<SettingDef>>)>,
        current: &AndroidSettings,
    ) -> Arc<Vec<SettingDef>> {
        let hash = current.defs_fingerprint();
        if let Some((cached_hash, defs)) = entry
            && *cached_hash == hash
        {
            return Arc::clone(defs);
        }
        let defs = Arc::new(current.setting_defs());
        *entry = Some((hash, Arc::clone(&defs)));
        defs
    }

    /// Cache key version: bump when the defs shape changes.
    /// Stale entries from an older shape never validate.
    const DEFS_CACHE_VERSION: u64 = 1;

    fn defs_fingerprint(&self) -> u64 {
        // FNV-1a over every field `setting_defs` reads; deterministic
        // across processes, unlike the default SipHash builder.
        fn push(hash: &mut u64, bytes: &[u8]) {
            for byte in bytes {
                *hash ^= u64::from(*byte);
                *hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        fn orientation_value(value: ScreenOrientation) -> u8 {
            match value {
                ScreenOrientation::Auto => 0,
                ScreenOrientation::Portrait => 1,
                ScreenOrientation::Landscape => 2,
            }
        }
        fn visibility_value(value: TouchOverlayVisibility) -> u8 {
            match value {
                TouchOverlayVisibility::Always => 0,
                TouchOverlayVisibility::Auto => 1,
                TouchOverlayVisibility::Hidden => 2,
            }
        }
        fn rumble_target_value(value: RumbleTarget) -> u8 {
            match value {
                RumbleTarget::Auto => 0,
                RumbleTarget::Handset => 1,
                RumbleTarget::Controller => 2,
            }
        }
        fn position_value(value: NotificationPosition) -> u8 {
            match value {
                NotificationPosition::BottomLeft => 0,
                NotificationPosition::BottomRight => 1,
                NotificationPosition::TopLeft => 2,
                NotificationPosition::TopRight => 3,
            }
        }
        fn area_mode_value(value: NotificationAreaMode) -> u8 {
            match value {
                NotificationAreaMode::Overlay => 0,
                NotificationAreaMode::Strip => 1,
            }
        }
        fn storage_policy_value(value: StoragePolicy) -> u8 {
            match value {
                StoragePolicy::Sidecar => 0,
                StoragePolicy::AppSharedData => 1,
                StoragePolicy::CustomDirectory => 2,
            }
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ Self::DEFS_CACHE_VERSION;
        push(&mut hash, &[u8::from(self.audio_muted)]);
        push(
            &mut hash,
            self.master_volume_percent.to_le_bytes().as_slice(),
        );
        push(&mut hash, self.latency_ms.to_le_bytes().as_slice());
        push(&mut hash, self.sample_rate.to_le_bytes().as_slice());
        push(&mut hash, &[u8::from(self.vsync)]);
        push(&mut hash, &[orientation_value(self.screen_orientation)]);
        push(&mut hash, &[storage_policy_value(self.storage_policy)]);
        push(&mut hash, &[visibility_value(self.overlay_visibility)]);
        push(&mut hash, &[self.overlay_opacity_percent]);
        push(&mut hash, &[self.overlay_scale_percent]);
        push(
            &mut hash,
            self.overlay_vertical_offset_percent
                .to_le_bytes()
                .as_slice(),
        );
        push(&mut hash, &[u8::from(self.overlay_haptics)]);
        push(&mut hash, &[u8::from(self.motion_enabled)]);
        push(&mut hash, &[u8::from(self.rumble_enabled)]);
        push(&mut hash, &[self.rumble_strength_percent]);
        push(&mut hash, &[rumble_target_value(self.rumble_target)]);
        push(&mut hash, &[u8::from(self.notifications_info_enabled)]);
        push(&mut hash, &[u8::from(self.notifications_warning_enabled)]);
        push(&mut hash, &[u8::from(self.notifications_error_enabled)]);
        push(&mut hash, &[position_value(self.notifications_position)]);
        push(&mut hash, &[area_mode_value(self.notifications_area_mode)]);
        push(&mut hash, &[u8::from(self.status_enabled)]);
        push(&mut hash, &[position_value(self.status_position)]);
        push(&mut hash, &[area_mode_value(self.status_area_mode)]);
        for choice in &self.system_choices {
            push(&mut hash, choice.system_id.to_string().as_bytes());
            push(&mut hash, choice.field_id.as_str().as_bytes());
            push(&mut hash, choice.label.as_bytes());
            push(&mut hash, choice.selected.as_str().as_bytes());
            for (id, label) in &choice.options {
                push(&mut hash, id.as_str().as_bytes());
                push(&mut hash, label.as_bytes());
            }
        }
        hash
    }
}
