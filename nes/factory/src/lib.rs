mod builder;
pub mod input_profiles;
mod settings;

use std::rc::Rc;

use nerust_core_traits::{
    audio::AudioBackend,
    factory::{
        CoreFactory, CoreParts, FactoryError, SystemDefaults,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
        load::{
            DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest,
            SystemLoadOptions, SystemLoadOptionsSchema,
        },
        settings::FactorySettingsView,
    },
    identity::SystemId,
};
use nerust_input_traits::{
    Controller, ControllerCollection, ControllerProfile, EmuInput, GuiInput, ProfileId,
};
use nerust_nes_settings::NesSettings;

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
        // Build controller devices per occupied port.
        let mut devices: Vec<Box<dyn Controller + Send>> = Vec::new();
        for (_, ctrl_opt) in &assignments.slots {
            let profile = match ctrl_opt {
                Some(p) => p,
                None => continue,
            };
            let pid = profile.profile_id();
            if pid == ProfileId::new("nes.famicom") {
                devices.push(Box::new(nerust_nes_device::famicom_set::FamicomPadP1::new()));
                devices.push(Box::new(nerust_nes_device::famicom_set::FamicomPadP2::new()));
            } else if pid == ProfileId::new("nes.standard_pad") {
                devices.push(Box::new(nerust_nes_device::standard_pad::StandardPad::new(
                    0x1F,
                )));
            }
        }
        let controller_collection = ControllerCollection::new(devices);
        let resources = input_factory
            .create_split(&controller_collection)
            .map_err(|e| FactoryError::Create(e.to_string()))?;
        let gui_input = GuiInput::from_split(&resources.split);
        let emu_input = EmuInput::from_split(&resources.split);
        builder::create_core_and_adapter(
            view,
            speaker,
            gui_input,
            emu_input,
            resources.field_map,
            controller_collection,
        )
    }

    fn probe_media(&self, media: &MediaObject) -> bool {
        media.bytes.len() >= 4 && media.bytes[..4] == [0x4E, 0x45, 0x53, 0x1A]
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
        let deref_view = view
            .system_config
            .as_deref_mut()
            .ok_or(FactoryError::InvalidSettings)?;
        let nes = deref_view
            .downcast_mut::<NesSettings>()
            .ok_or(FactoryError::InvalidSettings)?;
        settings::apply_nes_settings_choice_inner(nes, field, choice)?;
        Ok(())
    }

    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        let deref_view = view
            .system_config
            .as_deref()
            .ok_or(FactoryError::InvalidSettings)?;
        let nes = deref_view
            .downcast_ref::<NesSettings>()
            .ok_or(FactoryError::InvalidSettings)?;
        settings::resolve_nes_load_request_inner(nes, &view.language, options)
    }

    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        CommandLineOptions::default().into()
    }

    fn input_system_factory(&self) -> &dyn nerust_input_traits::InputSystemFactory {
        self
    }

    fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
        NesLoadOptionsSchema.into()
    }

    fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
        Some(self)
    }
}

impl SystemDefaults for NesFactory {
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        Some(Box::new(NesSettings::default()))
    }

    fn resolve_label(&self, label_id: &str, language: &str) -> Option<String> {
        let localized = |en: &str, ja: &str| -> String {
            match language {
                "ja" => ja.to_string(),
                _ => en.to_string(),
            }
        };
        match label_id {
            "nes.video.filter" => Some(localized("Filter", "フィルター")),
            "nes.filter.none" => Some(localized("None", "なし")),
            "nes.filter.ntsc_composite" => Some(localized("NTSC Composite", "NTSC コンポジット")),
            "nes.filter.ntsc_svideo" => Some(localized("NTSC S-Video", "NTSC S-ビデオ")),
            "nes.filter.ntsc_rgb" => Some(localized("NTSC RGB", "NTSC RGB")),
            "nes.core.mmc3_irq_variant" => {
                Some(localized("MMC3 IRQ Variant", "MMC3 IRQ バリアント"))
            }
            "nes.mmc3.auto" => Some(localized("Auto", "自動")),
            "nes.mmc3.sharp" => Some(localized("Sharp", "Sharp")),
            "nes.mmc3.nec" => Some(localized("Nec", "Nec")),
            _ => None,
        }
    }

    fn default_input_attachment_id(&self) -> Option<&'static str> {
        Some("nes.attachment.player1")
    }

    fn default_input_control_prefix(&self) -> Option<&'static str> {
        Some("nes.control")
    }
}

#[derive(Default, clap::Args, Eq, PartialEq, Clone, Debug)]
struct CommandLineOptions {
    /// Override mapper 4 MMC3 IRQ behavior
    #[clap(long, value_enum)]
    mmc3_irq_variant: Option<Mmc3IrqVariant>,
}

impl SystemLoadOptions for CommandLineOptions {}

#[derive(Debug, Eq, PartialEq)]
struct NesLoadOptionsSchema;
impl SystemLoadOptionsSchema for NesLoadOptionsSchema {
    type Options = CommandLineOptions;
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
enum Mmc3IrqVariant {
    Sharp,
    Nec,
}

impl From<Mmc3IrqVariant> for nerust_nes_core::core_options::Mmc3IrqVariant {
    fn from(value: Mmc3IrqVariant) -> Self {
        match value {
            Mmc3IrqVariant::Sharp => Self::Sharp,
            Mmc3IrqVariant::Nec => Self::Nec,
        }
    }
}

pub fn nes_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    nerust_nes_device::nes_device_controller_profiles()
}

pub fn create_test_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
) -> Result<CoreParts, FactoryError> {
    let factory = NesFactory;
    factory.create_core_and_adapter(view, speaker)
}
