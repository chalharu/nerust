pub mod descriptor;
pub mod load;
pub mod settings;

use crate::SystemId;
use crate::audio::AudioBackend;
use crate::factory::descriptor::{
    SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use crate::factory::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use crate::factory::settings::FactorySettingsView;
use nerust_input_traits::SystemInputAdapter;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("core creation failed: {0}")]
    Create(String),
    #[error("invalid settings choice: {0}")]
    InvalidChoice(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
}

/// Raw parts produced by a system factory before EmuCore wrapping.
pub struct CoreParts {
    pub core: Box<dyn crate::ConsoleCore>,
    pub adapter: Box<dyn SystemInputAdapter>,
    pub render_profile: nerust_render_base::VideoRenderProfile,
    pub palette: Box<[u32; 256]>,
}

/// システム（NES/SNES）の全知識をカプセル化する factory。
///
/// frontend はこの trait を通じてのみシステムと対話する。
/// 各システムの実装は `factory/{nes,snes}/` クレートで行う。
///
/// `FactorySettingsView` を介して設定を受け取ることで、
/// gui/runtime の `SettingsSnapshot` への依存を回避している。
pub trait CoreFactory {
    fn system_id(&self) -> SystemId;

    fn display_name(&self) -> &'static str;

    fn probe_media(&self, media: &MediaObject) -> bool;

    fn system_descriptor(&self) -> SystemDescriptor;

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
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, FactoryError>;

    fn default_load_options(&self) -> SystemLoadOptions;

    fn create_core_and_adapter(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<CoreParts, FactoryError>;
}
