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
        GbaLoadOptions::default().into()
    }

    fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
        GbaLoadOptionsSchema.into()
    }

    fn create_core_and_adapter_with_assignments(
        &self,
        _view: &FactorySettingsView,
        _speaker: Box<dyn AudioBackend>,
        _assignments: &InputAssignments,
    ) -> Result<CoreParts, FactoryError> {
        // Phase 11 で実装
        todo!("create_core_and_adapter_with_assignments")
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
}
