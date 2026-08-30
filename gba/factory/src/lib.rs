mod builder;

use std::rc::Rc;
use std::sync::Arc;

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
use nerust_input_traits::{
    ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments, InputResources,
    InputSystemFactory, SlotInfo,
};

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

    fn probe_media(&self, _media: &MediaObject) -> bool {
        // Phase 4 で実装（ROMヘッダ検証）
        false
    }

    fn settings_page(&self, _view: &FactorySettingsView) -> SystemSettingsPageModel {
        // Phase 2 で実装
        SystemSettingsPageModel {
            fields: Arc::new([]),
        }
    }

    fn apply_settings_choice(
        &self,
        _view: &mut FactorySettingsView,
        _field: &SystemSettingsFieldId,
        _choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        // Phase 2 で実装
        Ok(())
    }

    fn resolve_load_request(
        &self,
        _view: &FactorySettingsView,
        _options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        // Phase 2 で実装
        Ok(ResolvedLoadRequest {
            options: Box::new(nerust_gba_core::core_options::GbaCoreOptions),
        })
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

    fn resolve_label(&self, _label_id: &str, _language: &str) -> Option<String> {
        // Phase 2 で実装
        None
    }

    fn default_input_attachment_id(&self) -> Option<&'static str> {
        Some("gba.attachment.player1")
    }

    fn default_input_control_prefix(&self) -> Option<&'static str> {
        Some("gba.control")
    }
}

impl nerust_input_traits::InputPorts for GbaFactory {
    fn slots(&self) -> &[SlotInfo] {
        // Phase 2 で実装
        &[]
    }

    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>> {
        gba_device_controller_profiles()
    }
}

impl InputSystemFactory for GbaFactory {
    fn default_assignments(&self) -> InputAssignments {
        // Phase 2 で実装
        InputAssignments { slots: Vec::new() }
    }

    fn create_split(
        &self,
        _controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        // Phase 2 で実装
        todo!("create_split")
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
