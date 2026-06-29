mod adapter;
mod builder;
mod input_state;
mod settings;

use nerust_core_traits::SystemId;
use nerust_core_traits::audio::AudioBackend;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::{
    descriptor::{
        SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
    },
    emu_core::EmuCore,
    factory::FactoryError,
    load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions},
};
use nerust_input_traits::SystemInputAdapter;

pub mod touch;

pub use nerust_gui_shell::factory::CoreFactory;

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
        settings: &SettingsSnapshot,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), FactoryError> {
        builder::create_core_and_adapter(settings, speaker)
    }

    fn probe_media(&self, _media: &MediaObject) -> bool {
        true
    }

    fn system_descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            input_topology: nerust_nes_controller::topology::input_topology_descriptor(),
        }
    }

    fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        settings::nes_settings_page(settings)
    }

    fn apply_settings_choice(
        &self,
        settings: &mut SettingsSnapshot,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        settings::apply_nes_settings_choice(settings, field, choice)
    }

    fn resolve_load_request(
        &self,
        settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        settings::resolve_nes_load_request(settings, options)
    }

    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }
}

pub fn create_test_core_and_adapter(
    settings: &SettingsSnapshot,
    speaker: Box<dyn AudioBackend>,
) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), FactoryError> {
    let factory = NesFactory;
    factory.create_core_and_adapter(settings, speaker)
}
