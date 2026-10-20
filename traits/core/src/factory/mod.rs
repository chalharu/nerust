pub mod descriptor;
pub mod load;
pub mod settings;

use std::collections::HashMap;

use nerust_input_traits::{
    AttachmentId, DigitalControlId, InputAssignments, InputSystemFactory, InputValue,
};
use thiserror::Error;

use crate::{audio::AudioBackend, identity::SystemId};

use self::{
    descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
    load::{DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest},
    settings::FactorySettingsView,
};

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("core creation failed: {0}")]
    Create(String),
    #[error("invalid settings choice: {0}")]
    InvalidChoice(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
    #[error("invalid settings snapshot")]
    InvalidSettings,
}

/// Session-owned GUI input endpoint, behind a port for ISP.
/// Concrete double-buffering lives in `nerust_emu_thread`; cores and
/// shells program to this narrow surface only.
pub trait GuiInputPort: Send {
    /// Write one absolute field of the staged GUI state.
    fn set(
        &mut self,
        field: usize,
        value: InputValue,
    ) -> Result<(), nerust_input_traits::BufferError>;
    /// Must be called every frame. Copies absolute state into shared.
    fn publish(&mut self);
    /// Reset staged state to neutral.
    fn clear(&mut self);
}

/// Raw parts produced by a system factory before EmuCore wrapping.
pub struct CoreParts {
    pub core: Box<dyn crate::ConsoleCore>,
    /// Session-owned audio backend: the emu thread (not the core) pushes
    /// through the rate-control filter into this. Factories start it;
    /// cores never see it.
    pub audio: Box<dyn crate::audio::AudioBackend>,
    pub gui_input: Box<dyn GuiInputPort>,
    /// (attachment, control) → absolute field index
    pub field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    pub render_profile: nerust_render_traits::VideoRenderProfile,
    pub palette: Box<[u32]>,
    pub host_peripherals: crate::peripheral::HostPeripheralHandles,
}

/// Factory encapsulating all per-system knowledge. Frontends talk to
/// systems only through this trait; settings arrive via
/// `FactorySettingsView` (no snapshot dependency).
pub trait CoreFactory: Send + Sync {
    fn system_id(&self) -> Box<dyn SystemId>;

    fn display_name(&self) -> &'static str;

    /// File extensions shown by frontend media pickers.
    /// Media acceptance must still be decided by [`Self::probe_media`].
    fn supported_extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn probe_media(&self, media: &MediaObject) -> bool;

    fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel;

    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError>;

    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError>;

    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions>;

    fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema>;

    fn create_core_and_adapter(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<CoreParts, FactoryError> {
        self.create_core_and_adapter_with_assignments(
            view,
            speaker,
            &self.input_system_factory().default_assignments(),
        )
    }

    /// Same as create_core_and_adapter but with custom controller assignments.
    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &InputAssignments,
    ) -> Result<CoreParts, FactoryError>;

    /// Presentation-only delta needing no core rebuild. Returns the
    /// replacement video assets, or `None` for the full rebuild path.
    /// Default `None` keeps today's rebuild behavior.
    fn presentation_swap_assets(
        &self,
        _before: &FactorySettingsView,
        _after: &FactorySettingsView,
    ) -> Option<(nerust_render_traits::VideoRenderProfile, Box<[u32]>)> {
        None
    }

    /// Returns this factory's input system factory for negotiation.
    fn input_system_factory(&self) -> &dyn InputSystemFactory;

    /// Access the [`SystemDefaults`] facet of this factory.
    ///
    /// Returns `None` for factories that only provide core creation
    /// (ISP separation — GUI integration methods live on `SystemDefaults`).
    fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
        None
    }
}

/// System-specific GUI integration defaults.
///
/// Separated from [`CoreFactory`] to respect ISP: factories that only
/// provide core creation are not forced to implement label resolution,
/// keyboard binding seeds, or default system settings.
pub trait SystemDefaults: Send + Sync {
    /// Default system-specific settings to seed into the shared settings map.
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        None
    }

    /// Resolve a system-specific label ID to a localized string.
    /// `language` is "ja" or "en". Returns None if unknown (display raw ID).
    fn resolve_label(&self, _label_id: &str, _language: &str) -> Option<String> {
        None
    }

    /// Attachment ID prefix for default keyboard bindings.
    fn default_input_attachment_id(&self) -> Option<&'static str> {
        None
    }

    /// Control ID prefix for default keyboard bindings.
    fn default_input_control_prefix(&self) -> Option<&'static str> {
        None
    }
}
