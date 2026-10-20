use std::rc::Rc;

use nerust_input_traits::{
    AttachmentId, ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments,
    InputPorts, InputResources, InputSystemFactory, ProfileId, SlotInfo,
};
use nerust_nes_core::input_types::NesInputBuffer;

impl InputPorts for crate::NesFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[
            SlotInfo {
                id: AttachmentId::new("nes.attachment.player1"),
                label: "Player 1",
            },
            SlotInfo {
                id: AttachmentId::new("nes.attachment.player2"),
                label: "Player 2",
            },
        ];
        SLOTS
    }
    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
        Box::new(nerust_nes_device::nes_device_controller_profiles())
    }
}

impl InputSystemFactory for crate::NesFactory {
    fn default_assignments(&self) -> InputAssignments {
        let famicom = self
            .controllers()
            .find(|p| p.profile_id() == ProfileId::new("nes.famicom"));
        InputAssignments::new(vec![
            (AttachmentId::new("nes.attachment.player1"), famicom),
            (AttachmentId::new("nes.attachment.player2"), None),
        ])
    }

    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        use nerust_nes_core::controller::NES_PORTS;

        let mut field_map = std::collections::HashMap::new();
        for (idx, port) in NES_PORTS.iter().enumerate() {
            let Some(dev) = controllers.get(idx) else {
                continue;
            };
            for entry in dev.field_entries(port) {
                field_map.insert((entry.attachment, entry.control), entry.field);
            }
        }
        if field_map.is_empty() {
            return Err(CreateSplitError::ControllerNotFound {
                controller: ProfileId::new("nes.standard_pad"),
            });
        }

        Ok(InputResources::new(
            field_map,
            Box::new(|| Box::<NesInputBuffer>::default()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_nes_device::standard_pad::StandardPad;

    #[test]
    fn slots_expose_two_players() {
        assert_eq!(crate::NesFactory.slots().len(), 2);
        assert_eq!(
            crate::NesFactory.resolve_slot("nes.attachment.player2"),
            Some(AttachmentId::new("nes.attachment.player2"))
        );
        assert_eq!(crate::NesFactory.resolve_slot("nope"), None);
    }

    #[test]
    fn default_assignment_prefers_famicom() {
        let assignments = crate::NesFactory.default_assignments();
        assert_eq!(assignments.slots().len(), 2);
        let first = assignments.slots()[0].1.as_ref().expect("p1 pad");
        assert_eq!(first.profile_id(), ProfileId::new("nes.famicom"));
        assert!(assignments.slots()[1].1.is_none());
    }

    #[test]
    fn split_maps_pad_fields_and_rejects_empty() {
        let controllers = ControllerCollection::new(vec![
            Box::new(StandardPad::default()),
            Box::new(StandardPad::default()),
        ]);
        let resources = crate::NesFactory.create_split(&controllers).unwrap();
        assert_eq!(resources.field_map().len(), 16);
        let buffer = (resources.new_buffer())();
        assert!(buffer.downcast_ref::<NesInputBuffer>().is_some());
        assert!(
            resources
                .probe_set(0, nerust_input_traits::InputValue::Digital(true))
                .is_ok()
        );
        let empty = ControllerCollection::new(vec![]);
        assert!(crate::NesFactory.create_split(&empty).is_err());
    }
}
