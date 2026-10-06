mod builder;
pub mod input_profiles;
mod settings;

use std::{collections::HashMap, rc::Rc};

use nerust_core_traits::{
    audio::AudioBackend,
    debugger::SpaceId,
    factory::{
        CoreFactory, CoreParts, FactoryError, SystemDefaults,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
        load::{
            DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest,
            SystemLoadOptions, SystemLoadOptionsSchema,
        },
        settings::{FactorySettingsView, Language},
    },
    identity::SystemId,
};
use nerust_input_traits::{
    AttachmentId, Controller, ControllerCollection, ControllerProfile, DigitalControlId, EmuInput,
    GuiInput, ProfileId,
};
use nerust_nes_settings::{NesSettings, NesVideoFilter};

#[derive(Debug)]
pub struct NesFactory;

impl CoreFactory for NesFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        Box::new(nerust_nes_core::rom_identity::NesSystemId)
    }

    fn display_name(&self) -> &'static str {
        "NES"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["nes"]
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

    fn headless_view(&self) -> Result<FactorySettingsView, FactoryError> {
        let mut system_config = self
            .as_system_defaults()
            .and_then(|defaults| defaults.default_system_settings())
            .ok_or(FactoryError::InvalidSettings)?;
        system_config
            .downcast_mut::<NesSettings>()
            .ok_or(FactoryError::InvalidSettings)?
            .video
            .filter = NesVideoFilter::None;
        Ok(FactorySettingsView {
            language: Language::English,
            system_config: Some(system_config),
        })
    }
}

/// Headless-test support: system-owned answers so frontends never name
/// core types or table positions.
impl NesFactory {
    /// Load options carrying a headless MMC3 override. The caller maps
    /// its own schema enum to [`Mmc3IrqVariant`]; precedence
    /// (explicit > saved setting) stays inside `resolve_load_request`.
    pub fn headless_load_options(mmc3: Option<Mmc3IrqVariant>) -> Box<dyn DynSystemLoadOptions> {
        CommandLineOptions {
            mmc3_irq_variant: mmc3,
        }
        .into()
    }

    /// Space id resolved by stable table `key` (e.g. `"wram"`). Ids come
    /// from the validated table itself, so a reorder can never silently
    /// mis-resolve; unknown keys return `None` for a loud caller error.
    pub fn space_id_for_key(key: &str) -> Option<SpaceId> {
        nerust_nes_core::debugger::NES_SPACE_TABLE
            .entries
            .iter()
            .find(|info| info.key == key)
            .map(|info| info.id)
    }

    /// Bit-to-field layout for absolute pad seeding. Bits follow the
    /// suite convention (A B Select Start Up Down Left Right per pad).
    /// Pad 2 Select/Start stay `None`: `FamicomPadP2` hardware has no
    /// such buttons. Any other missing field is a loud `Err`.
    pub fn test_pad_layout(
        field_map: &HashMap<(AttachmentId, DigitalControlId), usize>,
    ) -> Result<TestPadLayout, FactoryError> {
        const BUTTONS: [&str; 8] = [
            "nes.control.a",
            "nes.control.b",
            "nes.control.select",
            "nes.control.start",
            "nes.control.up",
            "nes.control.down",
            "nes.control.left",
            "nes.control.right",
        ];
        // Canonical source: `FamicomPadP1/P2::field_map`
        // (`nes/device/src/famicom_set.rs`).
        let lookup = |attachment: &'static str, control: &'static str| {
            field_map
                .get(&(
                    AttachmentId::new(attachment),
                    DigitalControlId::new(control),
                ))
                .copied()
        };
        let missing = |attachment: &'static str, control: &'static str| {
            FactoryError::Create(format!("test input field missing: {attachment}/{control}"))
        };
        let mut pad_fields = [[None; 8]; 2];
        for (pad, attachment) in ["nes.attachment.player1", "nes.attachment.player2"]
            .into_iter()
            .enumerate()
        {
            for (bit, control) in BUTTONS.into_iter().enumerate() {
                // Pad 2 has no Select/Start buttons; the suite drives
                // those bits as no-ops (the device masks them too).
                let optional = pad == 1 && (bit == 2 || bit == 3);
                pad_fields[pad][bit] = match lookup(attachment, control) {
                    Some(field) => Some(field),
                    None if optional => None,
                    None => return Err(missing(attachment, control)),
                };
            }
        }
        let mic_field = Some(
            lookup("nes.attachment.player2", "famicom.microphone")
                .ok_or_else(|| missing("nes.attachment.player2", "famicom.microphone"))?,
        );
        Ok(TestPadLayout {
            pad_fields,
            mic_field,
        })
    }
}

/// Absolute pad-seeding layout; see [`NesFactory::test_pad_layout`].
pub struct TestPadLayout {
    /// `pad_fields[pad][bit]`; `None` = hardware has no such button.
    pub pad_fields: [[Option<usize>; 8]; 2],
    /// Microphone field.
    pub mic_field: Option<usize>,
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
pub enum Mmc3IrqVariant {
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

#[cfg(test)]
mod media_tests {
    use nerust_core_traits::factory::CoreFactory;

    use super::NesFactory;

    #[test]
    fn reports_nes_file_extension() {
        assert_eq!(NesFactory.supported_extensions(), &["nes"]);
    }
}
