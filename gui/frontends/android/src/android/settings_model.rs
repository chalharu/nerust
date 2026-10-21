//! Settings model: Android-visible subset and cached field descriptors.
//! Pure data plus snapshot mapping; JNI and transport live elsewhere.

use std::sync::Arc;

use nerust_core_traits::identity::SystemId;
use nerust_system_traits::factory::{
    CoreFactory, FactoryError,
    descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind},
};

use nerust_gui_runtime::settings::SettingsSnapshot;
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
pub(super) use crate::bridge_contract::SETTINGS_SCHEMA_VERSION;

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

    pub(super) fn from_snapshot(
        snapshot: &SettingsSnapshot,
        factories: &[Arc<dyn CoreFactory>],
    ) -> Self {
        let language = snapshot.shared.general.language;
        let system_choices = factories
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
        factories: &[Arc<dyn CoreFactory>],
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
            let factory = factories
                .iter()
                .find(|factory| factory.system_id() == choice.system_id)
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
        super::settings_schema::defs(self)
    }

    pub(super) fn cached_setting_defs_locked(
        entry: &mut Option<(AndroidSettings, Arc<Vec<SettingDef>>)>,
        current: &AndroidSettings,
    ) -> Arc<Vec<SettingDef>> {
        if let Some((cached, defs)) = entry
            && cached == current
        {
            return Arc::clone(defs);
        }
        let defs = Arc::new(current.setting_defs());
        *entry = Some((current.clone(), Arc::clone(&defs)));
        defs
    }
}
