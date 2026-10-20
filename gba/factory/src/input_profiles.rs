use std::rc::Rc;

use nerust_gba_core::input_types::GbaInputBuffer;
use nerust_input_traits::{
    AttachmentId, ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments,
    InputPorts, InputResources, InputSystemFactory, ProfileId, SimplePort, SlotInfo,
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

    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
        Box::new(nerust_gba_device::gba_device_controller_profiles())
    }
}

impl InputSystemFactory for crate::GbaFactory {
    fn default_assignments(&self) -> InputAssignments {
        let standard = self
            .controllers()
            .find(|profile| profile.profile_id() == ProfileId::new("gba.standard_pad"));
        InputAssignments::new(vec![(GBA_ATTACHMENT, standard)])
    }

    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        if controllers.is_empty() {
            return Err(CreateSplitError::ControllerNotFound {
                controller: ProfileId::new("gba.standard_pad"),
            });
        }
        if controllers.len() != 1 {
            return Err(CreateSplitError::IncompatibleController {
                controller: ProfileId::new("gba.standard_pad"),
                slot: GBA_ATTACHMENT,
            });
        }

        let field_map = {
            let Some(dev) = controllers.get(0) else {
                return Err(CreateSplitError::ControllerNotFound {
                    controller: ProfileId::new("gba.standard_pad"),
                });
            };
            dev.field_entries(&GBA_PORT)
                .into_iter()
                .map(|entry| ((entry.attachment, entry.control), entry.field))
                .collect()
        };
        Ok(InputResources::new(
            field_map,
            Box::new(|| Box::<GbaInputBuffer>::default()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use nerust_gba_device::StandardPad;

    use super::*;

    #[test]
    fn default_assignment_uses_standard_pad() {
        let assignments = crate::GbaFactory.default_assignments();
        assert_eq!(assignments.slots().len(), 1);
        assert_eq!(assignments.slots()[0].0, GBA_ATTACHMENT);
        assert_eq!(
            assignments.slots()[0]
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
        assert_eq!(resources.field_map().len(), 10);
        let buffer = (resources.new_buffer())();
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
