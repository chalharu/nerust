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

impl crate::GbcFactory {
    pub(crate) fn create_split(
        &self,
        assignments: &InputAssignments,
    ) -> Result<
        nerust_input_traits::TypedInputResources<nerust_gbc_core::input_types::GbcInputBuffer>,
        crate::FactoryError,
    > {
        use nerust_input_traits::CreateSplitError;
        if assignments.slots().len() != 1 || assignments.slots()[0].0 != GBC_ATTACHMENT {
            return Err(crate::FactoryError::Create(
                "GBC requires exactly one player1 assignment".to_string(),
            ));
        }
        let (attachment, profile) = assignments.slots().first().ok_or_else(|| {
            crate::FactoryError::Create("GBC controller is not assigned".to_string())
        })?;
        let _ = attachment;
        let profile = profile.as_ref().ok_or_else(|| {
            crate::FactoryError::Create("GBC controller is not assigned".to_string())
        })?;
        if profile.profile_id() != ProfileId::new("gbc.standard_pad") {
            return Err(crate::FactoryError::Create(format!(
                "unsupported GBC controller: {}",
                profile.profile_id()
            )));
        }
        let map = nerust_input_traits::collect_first_wins(
            nerust_gbc_device::StandardPad::input_fields(&GBC_PORT)
                .into_iter()
                .map(|entry| ((entry.attachment, entry.control), entry.field)),
        );
        if map.is_empty() {
            return Err(crate::FactoryError::Create(
                CreateSplitError::ControllerNotFound {
                    controller: ProfileId::new("gbc.standard_pad"),
                }
                .to_string(),
            ));
        }
        Ok(nerust_input_traits::TypedInputResources::new(
            map,
            nerust_gbc_core::input_types::GbcInputBuffer::default,
        ))
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
