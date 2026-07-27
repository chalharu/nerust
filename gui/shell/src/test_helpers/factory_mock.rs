use std::sync::Arc;

use nerust_core_traits::{
    audio::AudioBackend,
    factory::{
        CoreFactory, FactoryError,
        load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
        settings::FactorySettingsView,
    },
    identity::SystemId,
};
use nerust_input_traits::{InputAssignments, InputSystemFactory};

use crate::test_helpers::{
    DummyOtherSystemId, DummySystemId, MockInputFactory, NoopCoreOptions, NoopSystemLoadOptions,
    build_test_core_parts,
};

pub(crate) struct MockFactory;
impl CoreFactory for MockFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        Box::new(DummySystemId)
    }
    fn display_name(&self) -> &'static str {
        "NES (test)"
    }
    fn create_core_and_adapter_with_assignments(
        &self,
        _: &FactorySettingsView,
        _speaker: Box<dyn AudioBackend>,
        _: &InputAssignments,
    ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
        Ok(build_test_core_parts())
    }
    fn probe_media(&self, _: &MediaObject) -> bool {
        true
    }
    fn settings_page(
        &self,
        _: &FactorySettingsView,
    ) -> nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
        nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }
    fn apply_settings_choice(
        &self,
        _: &mut FactorySettingsView,
        _: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
        _: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        Ok(())
    }
    fn resolve_load_request(
        &self,
        _: &FactorySettingsView,
        _: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        Ok(ResolvedLoadRequest {
            options: NoopCoreOptions.into(),
        })
    }
    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        NoopSystemLoadOptions.into()
    }
    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        static MOCK_INPUT: MockInputFactory = MockInputFactory;
        &MOCK_INPUT
    }
    fn load_options_schema(
        &self,
    ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
        unreachable!()
    }
}

pub(crate) struct AlternateMockFactory;
impl CoreFactory for AlternateMockFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        Box::new(DummyOtherSystemId)
    }
    fn display_name(&self) -> &'static str {
        "Alternate (test)"
    }
    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &InputAssignments,
    ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
        MockFactory.create_core_and_adapter_with_assignments(view, speaker, assignments)
    }
    fn probe_media(&self, media: &MediaObject) -> bool {
        MockFactory.probe_media(media)
    }
    fn settings_page(
        &self,
        view: &FactorySettingsView,
    ) -> nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
        MockFactory.settings_page(view)
    }
    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
        choice: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        MockFactory.apply_settings_choice(view, field, choice)
    }
    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        MockFactory.resolve_load_request(view, options)
    }
    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        MockFactory.default_load_options()
    }
    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        MockFactory.input_system_factory()
    }
    fn load_options_schema(
        &self,
    ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
        MockFactory.load_options_schema()
    }
}
