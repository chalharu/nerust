pub mod descriptor;
pub mod load;
pub mod settings;

use std::collections::HashMap;

use nerust_input_traits::{
    AttachmentId, DigitalControlId, GuiInput, InputAssignments, InputSystemFactory,
};
use thiserror::Error;

use crate::{
    audio::AudioBackend,
    factory::{
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
        load::{
            DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest,
        },
        settings::FactorySettingsView,
    },
    identity::SystemId,
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

/// Raw parts produced by a system factory before EmuCore wrapping.
pub struct CoreParts {
    pub core: Box<dyn crate::ConsoleCore>,
    pub gui_input: GuiInput,
    /// (attachment, control) → absolute field index
    pub field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    pub render_profile: nerust_render_traits::VideoRenderProfile,
    pub palette: Box<[u32]>,
}

/// システム（NES/SNES）の全知識をカプセル化する factory。
///
/// frontend はこの trait を通じてのみシステムと対話する。
/// 各システムの実装は `factory/{nes,snes}/` クレートで行う。
///
/// `FactorySettingsView` を介して設定を受け取ることで、
/// gui/runtime の `SettingsSnapshot` への依存を回避している。
pub trait CoreFactory: Send + Sync {
    fn system_id(&self) -> SystemId;

    fn display_name(&self) -> &'static str;

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

    /// Returns this factory's input system factory for negotiation.
    fn input_system_factory(&self) -> &dyn InputSystemFactory;

    // -- Optional system presentation methods.
    //    Logically part of SystemDefaults; bridged here until call sites
    //    migrate to as_system_defaults() for full ISP separation.

    /// Access the SystemDefaults facet, if this factory provides one.
    fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
        None
    }

    /// Default system-specific settings to seed into the shared settings map.
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        self.as_system_defaults()
            .and_then(|d| d.default_system_settings())
    }

    /// Resolve a system-specific label ID to a localized string.
    /// `language` is "ja" or "en". Returns None if unknown (display raw ID).
    fn resolve_label(&self, _label_id: &str, _language: &str) -> Option<String> {
        self.as_system_defaults()
            .and_then(|d| d.resolve_label(_label_id, _language))
    }

    /// Attachment ID prefix for default keyboard bindings.
    fn default_input_attachment_id(&self) -> Option<&'static str> {
        self.as_system_defaults()
            .and_then(|d| d.default_input_attachment_id())
    }

    /// Control ID prefix for default keyboard bindings.
    fn default_input_control_prefix(&self) -> Option<&'static str> {
        self.as_system_defaults()
            .and_then(|d| d.default_input_control_prefix())
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
