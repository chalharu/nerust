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
use nerust_gba_settings::GbaSettings;
use nerust_input_traits::{ControllerProfile, InputAssignments, InputSystemFactory};

pub fn gba_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    nerust_gba_device::gba_device_controller_profiles()
}

#[derive(Debug)]
pub struct GbaFactory;

impl CoreFactory for GbaFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        Box::new(nerust_gba_core::rom_identity::GbaSystemId)
    }

    fn display_name(&self) -> &'static str {
        "Game Boy Advance"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["gba"]
    }

    fn probe_media(&self, media: &MediaObject) -> bool {
        if let Some(header) = nerust_gba_core::cartridge::header::GbaHeader::parse(&media.bytes) {
            header.logo_valid && header.fixed_valid && header.complement_valid
        } else {
            false
        }
    }

    fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel {
        settings::gba_settings_page(view)
    }

    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        settings::apply_gba_settings_choice(view, field, choice)
    }

    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        settings::resolve_gba_load_request(view, options)
    }

    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        GbaLoadOptions.into()
    }

    fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
        GbaLoadOptionsSchema.into()
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

impl SystemDefaults for GbaFactory {
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        Some(Box::new(GbaSettings::default()))
    }

    fn resolve_label(&self, label_id: &str, language: &str) -> Option<String> {
        labels::resolve(label_id, language)
    }

    fn default_input_attachment_id(&self) -> Option<&'static str> {
        Some("gba.attachment.player1")
    }

    fn default_input_control_prefix(&self) -> Option<&'static str> {
        Some("gba.control")
    }
}

#[derive(Default, clap::Args, Eq, PartialEq, Clone, Debug)]
struct GbaLoadOptions;

impl SystemLoadOptions for GbaLoadOptions {}

#[derive(Debug, Eq, PartialEq)]
struct GbaLoadOptionsSchema;

impl SystemLoadOptionsSchema for GbaLoadOptionsSchema {
    type Options = GbaLoadOptions;
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::factory::SystemDefaults;

    use super::*;

    #[test]
    fn default_system_settings_returns_gba_settings() {
        let factory = GbaFactory;
        let settings = factory.default_system_settings();
        assert!(settings.is_some());
    }

    #[test]
    fn resolve_label_returns_none_for_unknown_id() {
        let factory = GbaFactory;
        assert_eq!(factory.resolve_label("any.id", "en"), None);
    }

    fn minimal_gba_rom() -> Vec<u8> {
        let mut rom = vec![0; 0xC0];
        nerust_gba_core::cartridge::header::finalize_test_gba_rom(&mut rom);
        rom
    }

    fn minimal_gbc_rom() -> Vec<u8> {
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
        rom
    }

    #[test]
    fn probe_media_accepts_minimal_gba_rom() {
        assert!(GbaFactory.probe_media(&MediaObject::new(None, minimal_gba_rom())));
    }

    #[test]
    fn probe_media_rejects_non_gba_media() {
        assert!(!GbaFactory.probe_media(&MediaObject::new(None, b"NES\x1a".to_vec())));
        assert!(!GbaFactory.probe_media(&MediaObject::new(None, minimal_gbc_rom())));
        assert!(!GbaFactory.probe_media(&MediaObject::new(None, vec![])));
        assert!(!GbaFactory.probe_media(&MediaObject::new(None, vec![0; 0xBF])));
        assert!(!GbaFactory.probe_media(&MediaObject::new(None, vec![0; 0xC0])));
    }
}
