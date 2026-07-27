use std::{
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use nerust_input_traits::{
    BufferError, ControlInfo, ControllerCollection, ControllerProfile, CreateSplitError, GuiInput,
    InputAssignments, InputPorts, InputResources, InputSplit, InputStateBuffer, InputSystemFactory,
    InputValue, PortSet, ProfileId, SlotInfo,
};

use crate::test_helpers::TEST_SLOT_P1;

#[derive(Debug, Default)]
pub(crate) struct TestInputBuffer(pub [u8; 2]);

impl InputStateBuffer for TestInputBuffer {
    fn set(&mut self, _field: usize, _value: InputValue) -> Result<(), BufferError> {
        Ok(())
    }
    fn clear(&mut self) {
        self.0 = [0; 2];
    }
    fn copy_state(&mut self, other: &dyn InputStateBuffer) {
        if let Some(src) = other.downcast_ref::<TestInputBuffer>() {
            self.0 = src.0;
        }
    }
}

pub(crate) fn test_input_resources() -> (GuiInput, InputSplit) {
    let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
        Arc::new(Mutex::new(Box::<TestInputBuffer>::default()));
    let flag = Arc::new(AtomicBool::new(false));
    let gui = GuiInput::new(
        Arc::clone(&shared),
        Arc::clone(&flag),
        Box::new(|| Box::<TestInputBuffer>::default()),
    );
    let split = InputSplit {
        shared: Arc::clone(&shared),
        flag: Arc::clone(&flag),
        new_buffer: Box::new(|| Box::<TestInputBuffer>::default()),
    };
    (gui, split)
}

#[derive(Debug)]
pub(crate) struct MockSlotProfile;

impl ControllerProfile for MockSlotProfile {
    fn profile_id(&self) -> ProfileId {
        ProfileId::new("test.profile.p1")
    }
    fn label(&self) -> &'static str {
        "Test P1"
    }
    fn port_sets(&self) -> &[PortSet] {
        static PORTS: [PortSet; 1] = [PortSet {
            ports: &[TEST_SLOT_P1],
        }];
        &PORTS
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        static EMPTY: [ControlInfo; 0] = [];
        static GROUPS: [&[ControlInfo]; 1] = [&EMPTY];
        &GROUPS
    }
}

#[derive(Debug)]
pub(crate) struct MockInputFactory;
impl InputPorts for MockInputFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: [SlotInfo; 1] = [SlotInfo {
            id: TEST_SLOT_P1,
            label: "P1",
        }];
        &SLOTS
    }
    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>> {
        vec![Rc::new(MockSlotProfile)]
    }
}
impl InputSystemFactory for MockInputFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments {
            slots: vec![(TEST_SLOT_P1, None)],
        }
    }
    fn create_split(&self, _: &ControllerCollection) -> Result<InputResources, CreateSplitError> {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<TestInputBuffer>::default()));
        let flag = Arc::new(AtomicBool::new(false));
        Ok(InputResources {
            split: InputSplit {
                shared: Arc::clone(&shared),
                flag: Arc::clone(&flag),
                new_buffer: Box::new(|| Box::<TestInputBuffer>::default()),
            },
            field_map: std::collections::HashMap::new(),
        })
    }
}
