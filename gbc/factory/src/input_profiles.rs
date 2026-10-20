use std::rc::Rc;

use nerust_gbc_core::input_types::GbcInputBuffer;
use nerust_input_traits::{
    AttachmentId, ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments,
    InputPorts, InputResources, InputSplit, InputStateBuffer, InputSystemFactory, ProfileId,
    SimplePort, SlotInfo,
};

pub(crate) const GBC_ATTACHMENT: AttachmentId = AttachmentId::new("gbc.attachment.player1");
pub(crate) const GBC_PORT: SimplePort = SimplePort::new(0, "gbc.attachment.player1");

impl InputPorts for crate::GbcFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[SlotInfo {
            id: GBC_ATTACHMENT,
            label: "Player 1",
        }];
        SLOTS
    }

    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
        Box::new(nerust_gbc_device::gbc_device_controller_profiles())
    }
}

impl InputSystemFactory for crate::GbcFactory {
    fn default_assignments(&self) -> InputAssignments {
        let standard = self
            .controllers()
            .find(|profile| profile.profile_id() == ProfileId::new("gbc.standard_pad"));
        InputAssignments::new(vec![(GBC_ATTACHMENT, standard)])
    }

    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        if controllers.is_empty() {
            return Err(CreateSplitError::ControllerNotFound {
                controller: ProfileId::new("gbc.standard_pad"),
            });
        }
        if controllers.len() != 1 {
            return Err(CreateSplitError::IncompatibleController {
                controller: ProfileId::new("gbc.standard_pad"),
                slot: GBC_ATTACHMENT,
            });
        }

        let field_map = {
            let Some(dev) = controllers.get(0) else {
                return Err(CreateSplitError::ControllerNotFound {
                    controller: ProfileId::new("gbc.standard_pad"),
                });
            };
            let mut entries = Vec::new();
            dev.field_map(&GBC_PORT, &mut entries);
            entries
                .into_iter()
                .map(|entry| ((entry.attachment, entry.control), entry.field))
                .collect()
        };
        let shared: std::sync::Arc<std::sync::Mutex<Box<dyn InputStateBuffer>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Box::<GbcInputBuffer>::default()));
        let split = InputSplit::new(
            std::sync::Arc::clone(&shared),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Box::new(|| Box::<GbcInputBuffer>::default()),
        );
        Ok(InputResources::new(split, field_map))
    }
}

#[cfg(test)]
mod tests {
    use nerust_gbc_device::StandardPad;

    use super::*;

    #[test]
    fn default_assignment_uses_standard_pad() {
        let assignments = crate::GbcFactory.default_assignments();
        assert_eq!(assignments.slots().len(), 1);
        assert_eq!(assignments.slots()[0].0, GBC_ATTACHMENT);
        assert_eq!(
            assignments.slots()[0]
                .1
                .as_ref()
                .expect("standard pad")
                .profile_id(),
            ProfileId::new("gbc.standard_pad")
        );
    }

    #[test]
    fn split_maps_all_eight_controls_to_gbc_buffer() {
        let controllers = ControllerCollection::new(vec![Box::new(StandardPad::new())]);
        let resources = crate::GbcFactory.create_split(&controllers).unwrap();
        assert_eq!(resources.field_map().len(), 8);
        let buffer = (resources.split().new_buffer())();
        assert!(buffer.downcast_ref::<GbcInputBuffer>().is_some());
    }

    #[test]
    fn split_rejects_missing_or_extra_controllers() {
        let empty = ControllerCollection::new(vec![]);
        assert!(crate::GbcFactory.create_split(&empty).is_err());

        let extra = ControllerCollection::new(vec![
            Box::new(StandardPad::new()),
            Box::new(StandardPad::new()),
        ]);
        assert!(crate::GbcFactory.create_split(&extra).is_err());
    }
}
