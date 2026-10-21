use std::rc::Rc;

use nerust_input_traits::{
    AttachmentId, ControllerProfile, InputAssignments, InputPorts, InputSystemFactory, ProfileId,
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
}

#[cfg(test)]
mod tests {
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
}
