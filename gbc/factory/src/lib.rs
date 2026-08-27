mod builder;
mod input_profiles;
mod labels;
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
use nerust_gbc_settings::{GbcSettings, HardwareModel};
use nerust_input_traits::{ControllerProfile, InputAssignments, InputSystemFactory};

pub fn gbc_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    nerust_gbc_device::gbc_device_controller_profiles()
}

#[derive(Debug)]
pub struct GbcFactory;

impl CoreFactory for GbcFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        Box::new(nerust_gbc_core::rom_identity::GbcSystemId)
    }

    fn display_name(&self) -> &'static str {
        "Game Boy Color"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["gb", "gbc"]
    }

    fn probe_media(&self, media: &MediaObject) -> bool {
        nerust_gbc_core::cartridge_header::is_supported_rom(&media.bytes)
    }

    fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel {
        settings::gbc_settings_page(view)
    }

    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        settings::apply_gbc_settings_choice(view, field, choice)
    }

    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        settings::resolve_gbc_load_request(view, options)
    }

    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        GbcLoadOptions::default().into()
    }

    fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
        GbcLoadOptionsSchema.into()
    }

    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &InputAssignments,
    ) -> Result<CoreParts, FactoryError> {
        builder::create_core_and_adapter(view, speaker, assignments)
    }

    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        self
    }

    fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
        Some(self)
    }
}

impl SystemDefaults for GbcFactory {
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        Some(Box::new(GbcSettings::default()))
    }

    fn resolve_label(&self, label_id: &str, language: &str) -> Option<String> {
        labels::resolve(label_id, language)
    }

    fn default_input_attachment_id(&self) -> Option<&'static str> {
        Some("gbc.attachment.player1")
    }

    fn default_input_control_prefix(&self) -> Option<&'static str> {
        Some("gbc.control")
    }
}

#[derive(Default, clap::Args, Eq, PartialEq, Clone, Debug)]
struct GbcLoadOptions {
    #[arg(long = "gbc-hardware-model", value_name = "MODEL")]
    hardware_model: Option<HardwareModel>,
}

impl SystemLoadOptions for GbcLoadOptions {}

#[derive(Debug, Eq, PartialEq)]
struct GbcLoadOptionsSchema;

impl SystemLoadOptionsSchema for GbcLoadOptionsSchema {
    type Options = GbcLoadOptions;
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::factory::{CoreFactory, SystemDefaults, load::MediaObject};

    use super::*;

    #[test]
    fn default_system_settings_returns_gbc_settings() {
        let factory = GbcFactory;
        let settings = factory.default_system_settings();
        assert!(settings.is_some());
    }

    #[test]
    fn resolve_label_returns_none_for_unknown_id() {
        let factory = GbcFactory;
        assert_eq!(factory.resolve_label("any.id", "en"), None);
    }

    #[test]
    fn identifies_supported_gbc_rom() {
        let mut rom = vec![0; 0x8000];
        rom[0x0104..0x0134].copy_from_slice(&[
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        ]);
        let mut checksum = 0u8;
        for byte in &rom[0x0134..=0x014C] {
            checksum = checksum.wrapping_sub(*byte).wrapping_sub(1);
        }
        rom[0x014D] = checksum;

        assert!(GbcFactory.probe_media(&MediaObject::new(None, rom)));
        assert!(!GbcFactory.probe_media(&MediaObject::new(None, b"NES\x1a".to_vec())));
    }

    #[test]
    fn reports_gb_and_gbc_file_extensions() {
        assert_eq!(GbcFactory.supported_extensions(), &["gb", "gbc"]);
    }

    #[test]
    fn default_input_attachment_id_matches_device_profile() {
        let factory = GbcFactory;
        let attachment = factory
            .default_input_attachment_id()
            .expect("attachment id");
        assert_eq!(attachment, "gbc.attachment.player1");
    }

    #[test]
    fn default_input_control_prefix_matches_control_ids() {
        let factory = GbcFactory;
        let prefix = factory
            .default_input_control_prefix()
            .expect("control prefix");
        assert_eq!(prefix, "gbc.control");
    }
}
