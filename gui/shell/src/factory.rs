pub use nerust_core_traits::factory::{CoreParts, FactoryError};

use nerust_core_traits::SystemId;
use nerust_core_traits::audio::AudioBackend;
use nerust_core_traits::factory::descriptor::{
    SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use nerust_core_traits::factory::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_input_traits::SystemInputAdapter;

use crate::emu_core::EmuCore;

/// システム（NES/SNES）の全知識をカプセル化する factory。
///
/// frontend はこの trait を通じてのみシステムと対話する。
/// 各システムの実装は `factory/{nes,snes}/` クレートで行う。
pub trait CoreFactory {
    fn system_id(&self) -> SystemId;

    fn display_name(&self) -> &'static str;

    fn probe_media(&self, media: &MediaObject) -> bool;

    fn system_descriptor(&self) -> SystemDescriptor;

    fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel;

    fn apply_settings_choice(
        &self,
        settings: &mut SettingsSnapshot,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError>;

    fn resolve_load_request(
        &self,
        settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, FactoryError>;

    fn default_load_options(&self) -> SystemLoadOptions;

    fn create_core_and_adapter(
        &self,
        settings: &SettingsSnapshot,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), FactoryError>;
}
