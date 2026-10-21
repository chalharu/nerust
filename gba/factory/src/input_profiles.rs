use std::rc::Rc;

use nerust_input_traits::{
    AttachmentId, ControllerProfile, InputAssignments, InputPorts, InputSystemFactory, ProfileId,
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
}

#[cfg(test)]
mod tests {
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
}
