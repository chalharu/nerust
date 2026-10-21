use std::rc::Rc;

use nerust_emu_thread::{GuiInput, InputPort, InputSplit};
use nerust_input_traits::{
    BufferError, ControlInfo, ControllerProfile, InputAssignments, InputPorts, InputStateBuffer,
    InputSystemFactory, InputValue, PortSet, ProfileId, SlotInfo,
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
    fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
        if let Some(src) = other.downcast_ref::<TestInputBuffer>() {
            self.0 = src.0;
            Ok(())
        } else {
            Err(BufferError::MismatchedBuffer {
                expected: "TestInputBuffer",
                actual: std::any::type_name_of_val(other),
            })
        }
    }
}

pub(crate) fn test_input_resources() -> (GuiInput, InputSplit) {
    let split = InputSplit::spawn_typed(TestInputBuffer::default);
    let gui = split.gui_endpoint();
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
    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
        Box::new(vec![Rc::new(MockSlotProfile) as Rc<dyn ControllerProfile>].into_iter())
    }
}
impl InputSystemFactory for MockInputFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments::new(vec![(TEST_SLOT_P1, None)])
    }
}

#[cfg(test)]
mod input_mock_cov_tests {
    use crate::test_helpers::input_mock::*;

    #[test]
    fn buffer_copies_matching_and_rejects_foreign() {
        let mut buf = TestInputBuffer([1, 2]);
        buf.clear();
        assert_eq!(buf.0, [0, 0]);
        let src = TestInputBuffer([7, 8]);
        buf.copy_state(&src).expect("matching copies");
        assert_eq!(buf.0, [7, 8]);
        assert!(buf.set(0, InputValue::Digital(true)).is_ok());
        #[derive(Debug)]
        struct Foreign;
        impl InputStateBuffer for Foreign {
            fn set(&mut self, _: usize, _: InputValue) -> Result<(), BufferError> {
                Ok(())
            }
            fn clear(&mut self) {}
            fn copy_state(&mut self, _: &dyn InputStateBuffer) -> Result<(), BufferError> {
                Ok(())
            }
        }
        assert!(buf.copy_state(&Foreign).is_err());
    }

    #[test]
    fn factory_reports_slots_controllers_and_split() {
        let factory = MockInputFactory;
        assert_eq!(factory.slots().len(), 1);
        assert_eq!(factory.controllers().count(), 1);
        assert_eq!(factory.default_assignments().slots().len(), 1);
        let _ = test_input_resources();
        assert_eq!(MockSlotProfile.profile_id().as_str(), "test.profile.p1");
        assert_eq!(MockSlotProfile.label(), "Test P1");
        assert!(!MockSlotProfile.port_sets().is_empty());
    }
}
