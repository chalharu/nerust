use std::rc::Rc;

use nerust_gba_core::input_types::GbaInputBuffer;
use nerust_input_traits::{
    AttachmentId, ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments,
    InputPorts, InputResources, InputSplit, InputStateBuffer, InputSystemFactory, ProfileId,
    SimplePort, SlotInfo,
};

pub(crate) const GBA_ATTACHMENT: AttachmentId = AttachmentId::new("gba.attachment.player1");
pub(crate) const GBA_PORT: SimplePort = SimplePort::new(0, "gba.attachment.player1");

impl InputPorts for crate::GbaFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[SlotInfo {
            id: GBA_ATTACHMENT,
            label: "Player 1",
        }];
        SLOTS
    }

    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>> {
        nerust_gba_device::gba_device_controller_profiles()
    }
}

impl InputSystemFactory for crate::GbaFactory {
    fn default_assignments(&self) -> InputAssignments {
        let standard = self
            .controllers()
            .into_iter()
            .find(|profile| profile.profile_id() == ProfileId::new("gba.standard_pad"));
        InputAssignments {
            slots: vec![(GBA_ATTACHMENT, standard)],
        }
    }

    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        if controllers.device_count() != 1 {
            return Err(CreateSplitError::IncompatibleController {
                controller: format!("{} assigned controllers", controllers.device_count()),
                slot: GBA_ATTACHMENT.to_string(),
            });
        }

        let field_map = controllers.devices[0]
            .field_map(&GBA_PORT)
            .into_iter()
            .map(|(attachment, control, field)| ((attachment, control), field))
            .collect();
        let shared: std::sync::Arc<std::sync::Mutex<Box<dyn InputStateBuffer>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Box::<GbaInputBuffer>::default()));
        let split = InputSplit {
            shared: std::sync::Arc::clone(&shared),
            flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            new_buffer: Box::new(|| Box::<GbaInputBuffer>::default()),
        };
        Ok(InputResources { split, field_map })
    }
}

#[cfg(test)]
mod tests {
    use nerust_gba_device::StandardPad;

    use super::*;

    #[test]
    fn default_assignment_uses_standard_pad() {
        let assignments = crate::GbaFactory.default_assignments();
        assert_eq!(assignments.slots.len(), 1);
        assert_eq!(assignments.slots[0].0, GBA_ATTACHMENT);
        assert_eq!(
            assignments.slots[0]
                .1
                .as_ref()
                .expect("standard pad")
                .profile_id(),
            ProfileId::new("gba.standard_pad")
        );
    }

    #[test]
    fn split_maps_all_ten_controls_to_gba_buffer() {
        let controllers = ControllerCollection::new(vec![Box::new(StandardPad::new())]);
        let resources = crate::GbaFactory.create_split(&controllers).unwrap();
        assert_eq!(resources.field_map.len(), 10);
        let buffer = (resources.split.new_buffer)();
        assert!(buffer.downcast_ref::<GbaInputBuffer>().is_some());
    }

    #[test]
    fn split_rejects_missing_or_extra_controllers() {
        let empty = ControllerCollection::new(vec![]);
        assert!(crate::GbaFactory.create_split(&empty).is_err());

        let extra = ControllerCollection::new(vec![
            Box::new(StandardPad::new()),
            Box::new(StandardPad::new()),
        ]);
        assert!(crate::GbaFactory.create_split(&extra).is_err());
    }
}
