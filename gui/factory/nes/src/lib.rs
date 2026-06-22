mod adapter;
mod builder;
mod input_state;
mod settings;

use nerust_contract_core::input::SystemInputAdapter;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::descriptor::{
    SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use nerust_gui_shell::emu_core::EmuCore;
use nerust_gui_shell::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use nerust_input_schema::SystemId;

pub mod touch;

pub use nerust_gui_shell::factory::CoreFactory;

/// Opaque option bytes for MMC3 IRQ variant: "sharp".
pub const MMC3_OPTION_SHARP: &[u8] = b"sharp";
/// Opaque option bytes for MMC3 IRQ variant: "nec".
pub const MMC3_OPTION_NEC: &[u8] = b"nec";

pub struct NesFactory;

impl CoreFactory for NesFactory {
    fn create_core_and_adapter(
        &self,
        settings: &SettingsSnapshot,
    ) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), String> {
        builder::create_core_and_adapter(settings)
    }

    fn probe_media(&self, _media: &MediaObject) -> bool {
        true
    }

    fn system_descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            system_id: SystemId::Nes,
            input_topology: nerust_input_nes_runtime::topology::input_topology_descriptor(),
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
    ) -> Result<(), String> {
        settings::apply_nes_settings_choice(settings, field, choice)
    }

    fn resolve_load_request(
        &self,
        settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, String> {
        settings::resolve_nes_load_request(settings, options)
    }

    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }
}

pub fn create_test_core_and_adapter(
    settings: &SettingsSnapshot,
) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), String> {
    let factory = NesFactory;
    factory.create_core_and_adapter(settings)
}
