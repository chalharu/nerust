#![allow(dead_code)]

use std::sync::Arc;

use nerust_core_traits::{
    factory::{
        CoreFactory, CoreParts, FactoryError,
        descriptor::SystemSettingsPageModel,
        load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
        settings::FactorySettingsView,
    },
    identity::SystemId,
};
use nerust_input_traits::{
    AttachmentId, ControllerProfile, InputAssignments, InputPorts, InputResources,
    InputSystemFactory, PortSet, ProfileId, SlotInfo,
};

use super::SettingsViewModel;

nerust_core_traits::declare_system_id!(pub TestSystemId, "test");

#[derive(Debug)]
pub struct TestSlotProfile {
    id: ProfileId,
    label: &'static str,
    port_set: PortSet,
}

impl ControllerProfile for TestSlotProfile {
    fn profile_id(&self) -> ProfileId {
        self.id
    }
    fn label(&self) -> &'static str {
        self.label
    }
    fn port_sets(&self) -> &[PortSet] {
        std::slice::from_ref(&self.port_set)
    }
    fn port_groups(&self) -> &[&[nerust_input_traits::ControlInfo]] {
        static EMPTY_CTRLS: [nerust_input_traits::ControlInfo; 0] = [];
        static GROUPS: [&[nerust_input_traits::ControlInfo]; 1] = [&EMPTY_CTRLS];
        &GROUPS
    }
}

pub const P1_SLOT: AttachmentId = AttachmentId::new("test.slot.p1");
pub const P2_SLOT: AttachmentId = AttachmentId::new("test.slot.p2");

#[derive(Debug)]
pub struct TestInputFactory;

impl TestInputFactory {
    pub fn new() -> Self {
        Self
    }
}

impl InputPorts for TestInputFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: [SlotInfo; 2] = [
            SlotInfo {
                id: P1_SLOT,
                label: "P1",
            },
            SlotInfo {
                id: P2_SLOT,
                label: "P2",
            },
        ];
        &SLOTS
    }
    fn controllers(&self) -> Vec<std::rc::Rc<dyn ControllerProfile>> {
        static P1_PORTS: [AttachmentId; 1] = [P1_SLOT];
        static P2_PORTS: [AttachmentId; 1] = [P2_SLOT];
        use std::rc::Rc;
        vec![
            Rc::new(TestSlotProfile {
                id: ProfileId::new("test.ctrl.p1"),
                label: "Test P1",
                port_set: PortSet { ports: &P1_PORTS },
            }),
            Rc::new(TestSlotProfile {
                id: ProfileId::new("test.ctrl.p2"),
                label: "Test P2",
                port_set: PortSet { ports: &P2_PORTS },
            }),
        ]
    }
}

impl InputSystemFactory for TestInputFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments { slots: vec![] }
    }
    fn create_split(
        &self,
        _: &nerust_input_traits::ControllerCollection,
    ) -> Result<InputResources, nerust_input_traits::CreateSplitError> {
        unreachable!("not called in tests")
    }
}

#[derive(Debug)]
pub struct TestCoreFactory(pub TestInputFactory);

impl CoreFactory for TestCoreFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        Box::new(TestSystemId)
    }
    fn display_name(&self) -> &'static str {
        "Test"
    }
    fn create_core_and_adapter_with_assignments(
        &self,
        _: &FactorySettingsView,
        _: Box<dyn nerust_core_traits::audio::AudioBackend>,
        _: &InputAssignments,
    ) -> Result<CoreParts, FactoryError> {
        unreachable!("not called in tests")
    }
    fn probe_media(&self, _: &MediaObject) -> bool {
        false
    }
    fn settings_page(&self, _: &FactorySettingsView) -> SystemSettingsPageModel {
        SystemSettingsPageModel {
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
        unreachable!("not called in tests")
    }
    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        unreachable!("not called in tests")
    }
    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        &self.0
    }
    fn load_options_schema(
        &self,
    ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
        unreachable!("not called in tests")
    }
}

/// Helper to create a SettingsViewModel with a test factory.
pub fn test_vm() -> SettingsViewModel {
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_settings::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };
    let snapshot = SettingsSnapshot {
        shared: DesktopSharedSettings::default(),
        local: HostBackendLocalSettings::default(),
        app_state: DesktopAppState::default(),
    };
    let factory: Arc<dyn CoreFactory> = Arc::new(TestCoreFactory(TestInputFactory::new()));
    let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
        factory,
    ]));
    SettingsViewModel::new(snapshot, registry, Arc::new([]))
}
