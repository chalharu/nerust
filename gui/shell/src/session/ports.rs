//! Persistence ports: slot/autosave/mapper backends plus the manager-facing traits.
//! Platform adapters implement these; `fs` holds the desktop filesystem one.

use std::path::{Path, PathBuf};

use nerust_core_traits::identity::SystemIdentity;
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    thumbnail::ThumbnailSource,
};

use super::error::SlotOpFailure;

/// Persistence-facing view of the live core: slot bytes are stored verbatim.
/// The caller resolves the envelope at the session boundary; implemented for the live core.
pub trait StateExporter {
    /// Export raw core state bytes for slot storage.
    fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure>;
    /// Import raw core state bytes previously resolved by the caller.
    fn import_state(&self, data: Vec<u8>) -> Result<(), SlotOpFailure>;
    /// Preview pixels for the slot thumbnail, if a frame is available.
    fn export_preview(&self) -> Option<ThumbnailSource>;
    /// Identity the slot is filed under.
    fn export_identity(&self) -> Option<SystemIdentity>;
    /// Canonical media identity for slot filing; defaults to [`export_identity`](Self::export_identity).
    fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        self.export_identity()
    }
    /// Export the battery-backed mapper save, if the core holds one.
    fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure>;
    /// Import a battery-backed mapper save from disk.
    fn import_mapper(&self, bytes: Vec<u8>) -> Result<(), SlotOpFailure>;
}

/// Whether a save also moves the active-slot pointer. Explicit enum:
/// a bare `bool` at six call sites never says which way it flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlotActivation {
    /// Point the active slot at the saved slot.
    #[default]
    Activate,
    /// Leave the active-slot pointer alone.
    Leave,
}

/// One-bundle backend set for the persistence manager.
/// Construct literally at the composition root or in tests.
pub struct PersistenceBackends {
    pub slot: Box<dyn SlotBackend>,
    pub autosave: Box<dyn AutoSaveBackend>,
    pub mapper: Box<dyn MapperSaveBackend>,
}

/// Loader-facing session ports; load adapter depends on this, never the handle.
pub trait SessionPorts {
    /// Default load options for the active system.
    fn default_load_options(
        &self,
    ) -> Option<Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptions>>;
    /// Current settings snapshot.
    fn settings_snapshot(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot;
    /// Load resolved media.
    fn load_resolved(
        &mut self,
        media: nerust_core_traits::factory::load::MediaObject,
        resolved: nerust_core_traits::factory::load::ResolvedLoadRequest,
    ) -> Result<(), crate::load::RomLoaderError>;
    /// Resume after load.
    fn resume(&mut self);
    /// Activate a system before load.
    fn set_active_system(
        &mut self,
        system_id: &dyn nerust_core_traits::identity::SystemId,
    ) -> Result<(), crate::load::SystemActivationError>;
}

/// Plain-args notification view; single adapter at the settings boundary.
#[derive(Debug, Clone, Copy)]
pub struct NotificationView {
    /// Whether info rows display.
    pub info_enabled: bool,
    /// Whether warning rows display.
    pub warning_enabled: bool,
    /// Whether error rows display.
    pub error_enabled: bool,
    /// Display duration for info rows.
    pub info_duration_ms: u32,
    /// Display duration for warning rows.
    pub warning_duration_ms: u32,
    /// Display color for info rows.
    pub info_rgb: (u8, u8, u8),
    /// Display color for warning rows.
    pub warning_rgb: (u8, u8, u8),
    /// Display color for error rows.
    pub error_rgb: (u8, u8, u8),
    /// Whether errors also go modal.
    pub error_modal: bool,
}

/// Single settings-boundary adapter for notification views.
pub fn notification_view(
    settings: &nerust_settings_core::model::local::NotificationSettings,
) -> NotificationView {
    NotificationView {
        info_enabled: settings.info_enabled,
        warning_enabled: settings.warning_enabled,
        error_enabled: settings.error_enabled,
        info_duration_ms: settings.info_duration_ms,
        warning_duration_ms: settings.warning_duration_ms,
        info_rgb: settings.info_color.rgb(),
        warning_rgb: settings.warning_color.rgb(),
        error_rgb: settings.error_color.rgb(),
        error_modal: settings.error_modal,
    }
}

/// Plain-args chrome view; single adapter at the settings boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeView {
    /// Whether the status pill shows.
    pub status_enabled: bool,
    /// Corner for transient rows.
    pub position: nerust_settings_core::model::local::NotificationPosition,
    /// Corner for the status pill.
    pub status_position: nerust_settings_core::model::local::NotificationPosition,
    /// Layout mode for transient rows.
    pub area_mode: nerust_settings_core::model::local::NotificationAreaMode,
    /// Layout mode for the status pill.
    pub status_area_mode: nerust_settings_core::model::local::NotificationAreaMode,
}

/// Single settings-boundary adapter for chrome views.
pub fn chrome_view(
    settings: &nerust_settings_core::model::local::NotificationSettings,
) -> ChromeView {
    ChromeView {
        status_enabled: settings.status_enabled,
        position: settings.position,
        status_position: settings.status_position,
        area_mode: settings.area_mode,
        status_area_mode: settings.status_area_mode,
    }
}

/// Platform abstraction for state slot I/O (Desktop fs / Android SAF).
pub trait SlotBackend: Send {
    fn scan(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError>;
    fn allocate_next_id(&self, dir: &Path) -> Result<u64, PersistenceError>;
    fn write_slot(
        &self,
        dir: &Path,
        slot_id: u64,
        data: &[u8],
        identity: &SystemIdentity,
        thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError>;
    fn read_slot(
        &self,
        dir: &Path,
        slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError>;
    fn delete_slot(&self, dir: &Path, slot_id: u64) -> Result<(), PersistenceError>;
}

/// Platform abstraction for autosave I/O.
pub trait AutoSaveBackend: Send {
    fn write_autosave(
        &self,
        dir: &Path,
        data: &[u8],
        identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError>;
    fn read_autosave(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError>;
    fn delete_autosave(&self, dir: &Path) -> Result<(), PersistenceError>;
    /// Quarantine the hidden autosave aside instead of deleting it.
    /// Returns the quarantine path, or `None` when nothing was kept.
    fn quarantine_autosave(&self, dir: &Path) -> Result<Option<PathBuf>, PersistenceError> {
        self.delete_autosave(dir).map(|_| None)
    }
}

/// Platform abstraction for mapper save I/O.
pub trait MapperSaveBackend: Send {
    fn read_mapper_save(&self, path: &Path) -> Result<Option<Vec<u8>>, PersistenceError>;
    fn write_mapper_save(&self, path: &Path, data: &[u8]) -> Result<(), PersistenceError>;
    fn write_recovery_mapper_save(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<PathBuf, PersistenceError>;
}
