mod builder;
pub mod input_profiles;
mod settings;

use clap::{Arg, ArgMatches, Command};

use nerust_core_traits::SystemId;
use nerust_core_traits::audio::AudioBackend;
use nerust_core_traits::factory::cli::CliProvider;
use nerust_core_traits::factory::descriptor::{
    SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use nerust_core_traits::factory::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use nerust_core_traits::factory::settings::FactorySettingsView;
use nerust_core_traits::factory::{CoreFactory, CoreParts, FactoryError};
use nerust_input_traits::{EmuInput, GuiInput};

/// Opaque option bytes for MMC3 IRQ variant: "sharp".
pub const MMC3_OPTION_SHARP: &[u8] = b"sharp";
/// Opaque option bytes for MMC3 IRQ variant: "nec".
pub const MMC3_OPTION_NEC: &[u8] = b"nec";

#[derive(Debug)]
pub struct NesFactory;

impl CoreFactory for NesFactory {
    fn system_id(&self) -> SystemId {
        SystemId::new("nes")
    }

    fn display_name(&self) -> &'static str {
        "NES"
    }

    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &nerust_input_traits::InputAssignments,
    ) -> Result<CoreParts, FactoryError> {
        let input_factory: &dyn nerust_input_traits::InputSystemFactory = self;
        let resources = input_factory
            .create_split(assignments)
            .map_err(|e| FactoryError::Create(e.to_string()))?;
        let gui_input = GuiInput::from_split(&resources.split);
        let emu_input = EmuInput::from_split(&resources.split);
        // Compute controller mask from assignments
        let mut controller_mask = [0xFFu8; 2];
        for (slot_id, ctrl_opt) in &assignments.slots {
            let ctrl_id = match ctrl_opt {
                Some(id) => id.as_str(),
                None => continue,
            };
            if ctrl_id == "nes.famicom" && slot_id == "player1" {
                // FamicomSet: P2 has no Select/Start (bits 2,3 masked out)
                controller_mask[1] = 0b11110011;
            }
        }
        builder::create_core_and_adapter(
            view,
            speaker,
            gui_input,
            emu_input,
            resources.field_map,
            controller_mask,
        )
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

    fn input_system_factory(&self) -> &dyn nerust_input_traits::InputSystemFactory {
        self
    }
}

impl CliProvider for NesFactory {
    fn extend_command(&self, cmd: Command) -> Command {
        cmd.arg(
            Arg::new("mmc3-irq-variant")
                .long("mmc3-irq-variant")
                .value_parser(["sharp", "nec"])
                .help("Override mapper 4 MMC3 IRQ behavior"),
        )
    }

    fn parse_core_options(&self, matches: &ArgMatches) -> Vec<u8> {
        match matches
            .get_one::<String>("mmc3-irq-variant")
            .map(String::as_str)
        {
            Some("sharp") => MMC3_OPTION_SHARP.to_vec(),
            Some("nec") => MMC3_OPTION_NEC.to_vec(),
            _ => Vec::new(),
        }
    }
}

pub fn create_test_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
) -> Result<CoreParts, FactoryError> {
    let factory = NesFactory;
    factory.create_core_and_adapter(view, speaker)
}
