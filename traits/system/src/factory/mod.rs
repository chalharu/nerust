pub mod descriptor;
pub mod load;
pub mod settings;

use nerust_core_traits::{audio::AudioBackend, identity::SystemId};
use nerust_input_traits::{GuiInputPort, InputAssignments, InputSystemFactory, UnifiedFieldMap};
use thiserror::Error;

/// Resolve a control without dropping its digital/analog kind.
pub fn resolve_unified_field(
    map: &UnifiedFieldMap,
    attachment: nerust_input_traits::AttachmentId,
    control: impl Into<nerust_input_traits::ControlId>,
) -> Option<usize> {
    map.get(&(attachment, control.into())).copied()
}

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

/// Session resources assembled outside the core execution contract.
pub struct CoreParts {
    pub core: Box<dyn nerust_core_traits::ConsoleCore>,
    pub audio: Box<dyn AudioBackend>,
    pub gui_input: Box<dyn GuiInputPort>,
    pub field_map: UnifiedFieldMap,
    pub render_profile: nerust_render_traits::VideoRenderProfile,
    pub palette: Box<[u32]>,
    pub host_peripherals: nerust_core_traits::peripheral::HostPeripheralHandles,
}

/// Application-level factory. CLI and GUI contracts never flow back into cores.
pub trait CoreFactory: Send + Sync {
    fn system_id(&self) -> Box<dyn SystemId>;
    fn display_name(&self) -> &'static str;
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
    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &InputAssignments,
    ) -> Result<CoreParts, FactoryError>;
    fn presentation_swap_assets(
        &self,
        _before: &FactorySettingsView,
        _after: &FactorySettingsView,
    ) -> Option<(nerust_render_traits::VideoRenderProfile, Box<[u32]>)> {
        None
    }
    fn input_system_factory(&self) -> &dyn InputSystemFactory;
    fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
        None
    }
}

pub trait SystemDefaults: Send + Sync {
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        None
    }
    fn resolve_label(&self, _label_id: &str, _language: &str) -> Option<String> {
        None
    }
    fn default_input_attachment_id(&self) -> Option<&'static str> {
        None
    }
    fn default_input_control_prefix(&self) -> Option<&'static str> {
        None
    }
}
