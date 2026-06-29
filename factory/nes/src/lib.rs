mod adapter;
mod builder;
mod settings;

use nerust_core_traits::SystemId;
use nerust_core_traits::audio::AudioBackend;
use nerust_core_traits::factory::descriptor::{
    SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use nerust_core_traits::factory::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use nerust_core_traits::factory::settings::FactorySettingsView;
use nerust_core_traits::factory::{CoreFactory, CoreParts, FactoryError};

/// Opaque option bytes for MMC3 IRQ variant: "sharp".
pub const MMC3_OPTION_SHARP: &[u8] = b"sharp";
/// Opaque option bytes for MMC3 IRQ variant: "nec".
pub const MMC3_OPTION_NEC: &[u8] = b"nec";

pub struct NesFactory;

impl CoreFactory for NesFactory {
    fn system_id(&self) -> SystemId {
        SystemId::new("nes")
    }

    fn display_name(&self) -> &'static str {
        "NES"
    }

    fn create_core_and_adapter(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<CoreParts, FactoryError> {
        builder::create_core_and_adapter(view, speaker)
    }

    fn probe_media(&self, _media: &MediaObject) -> bool {
        true
    }

    fn system_descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            input_topology: nerust_nes_controller::topology::input_topology_descriptor(),
        }
    }

    fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel {
        settings::nes_settings_page(view)
    }

    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        let mut s = settings::deserialize_settings(&view.system_config_bytes);
        settings::apply_nes_settings_choice_inner(&mut s, field, choice)?;
        view.system_config_bytes = settings::serialize_settings(&s);
        Ok(())
    }

    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        let nes = settings::deserialize_settings(&view.system_config_bytes);
        settings::resolve_nes_load_request_inner(&nes, &view.language, options)
    }

    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }
}

pub fn create_test_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
) -> Result<CoreParts, FactoryError> {
    let factory = NesFactory;
    factory.create_core_and_adapter(view, speaker)
}
