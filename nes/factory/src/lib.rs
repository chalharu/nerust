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
use nerust_input_traits::{ControllerProfile, EmuInput, GuiInput};
use nerust_nes_core::controller::Controller;

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
        // Build one controller per occupied port, in slot order.
        // Multi-port controllers (e.g. FamicomSet) occupy more than one slot.
        let controllers = input_factory.controllers();
        // (slot_position, group_index, ctrl_id)
        let mut occupied: Vec<(usize, usize, &str)> = Vec::new();
        for (slot_pos, (slot_id, ctrl_opt)) in assignments.slots.iter().enumerate() {
            let ctrl_id = match ctrl_opt {
                Some(id) => id.as_str(),
                None => continue,
            };
            let Some(profile) = controllers.iter().find(|p| p.id() == ctrl_id) else {
                continue;
            };
            for ps in profile.port_sets() {
                if let Some(gi) = ps.ports.iter().position(|&p| p == slot_id) {
                    occupied.push((slot_pos, gi, ctrl_id));
                    // Also occupy other ports in the same set
                    for (other_gi, &port) in ps.ports.iter().enumerate() {
                        if port == slot_id {
                            continue;
                        }
                        if let Some(other_pos) =
                            assignments.slots.iter().position(|(s, _)| s == port)
                        {
                            occupied.push((other_pos, other_gi, ctrl_id));
                        }
                    }
                }
            }
        }
        // Deduplicate by slot position
        occupied.sort_by_key(|&(pos, _, _)| pos);
        occupied.dedup_by_key(|&mut (pos, _, _)| pos);
        let devices: Vec<Box<dyn Controller + Send>> = occupied
            .into_iter()
            .map(|(slot_pos, gi, ctrl_id)| {
                let ctrl: Box<dyn Controller + Send> = match (ctrl_id, gi) {
                    ("nes.famicom", 0) => {
                        Box::new(nerust_nes_device::famicom_set::FamicomPadP1::new())
                    }
                    ("nes.famicom", 1) => {
                        Box::new(nerust_nes_device::famicom_set::FamicomPadP2::new())
                    }
                    ("nes.standard_pad", _) => {
                        // Slot 0 (P1) gets open_bus_mask 3, slot 1 (P2) gets mask 1
                        let mask = if slot_pos == 0 { 3 } else { 1 };
                        Box::new(nerust_nes_device::standard_pad::StandardPad::new(mask))
                    }
                    _ => unreachable!(),
                };
                ctrl
            })
            .collect();
        builder::create_core_and_adapter(
            view,
            speaker,
            gui_input,
            emu_input,
            resources.field_map,
            devices,
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

pub fn nes_device_controller_profiles() -> Vec<Box<dyn ControllerProfile>> {
    nerust_nes_device::nes_device_controller_profiles()
}

pub fn create_test_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
) -> Result<CoreParts, FactoryError> {
    let factory = NesFactory;
    factory.create_core_and_adapter(view, speaker)
}
