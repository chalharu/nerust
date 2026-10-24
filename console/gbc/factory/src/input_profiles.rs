use std::rc::Rc;

use nerust_input_traits::{
    AttachmentId, ControllerProfile, CreateSplitError, InputAssignments, InputPorts,
    InputSystemFactory, ProfileId, SimplePort, SlotInfo,
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
        CreateSplitError,
    > {
        use nerust_input_traits::Controller as _;
        let mut plan = assignments.placement_plan(self)?;
        let placement = plan
            .drain(..)
            .next()
            .ok_or(CreateSplitError::EmptyAssignments)?;
        let Some((profile, group)) = placement else {
            return Err(CreateSplitError::EmptyAssignments);
        };
        let device = device_for_profile(&profile, group)?;
        let mut map = nerust_input_traits::UnifiedFieldMap::new();
        for entry in device.field_entries(&GBC_PORT) {
            let _ = map.insert((entry.attachment, entry.control), entry.field);
        }
        if map.is_empty() {
            return Err(CreateSplitError::EmptyAssignments);
        }
        Ok(nerust_input_traits::TypedInputResources::new(
            map,
            nerust_gbc_core::input_types::GbcInputBuffer::default,
        ))
    }
}

/// Single-profile device registry: the canonical placement profile selects the device.
/// Unknown profiles cannot arrive through [`placement_plan`](InputAssignments::placement_plan);
/// the fallback names the actual offender for forward compatibility.
fn device_for_profile(
    profile: &Rc<dyn ControllerProfile>,
    _group: usize,
) -> Result<nerust_gbc_device::StandardPad, CreateSplitError> {
    if profile.profile_id() == ProfileId::new("gbc.standard_pad") {
        Ok(nerust_gbc_device::StandardPad::new())
    } else {
        Err(CreateSplitError::ControllerNotFound {
            controller: profile.profile_id(),
        })
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
    fn split_maps_eight_fields_and_rejects_bad_assignments() {
        let resources = crate::GbcFactory
            .create_split(&crate::GbcFactory.default_assignments())
            .unwrap();
        assert_eq!(resources.field_map().len(), 8);
        let empty = InputAssignments::new(vec![(GBC_ATTACHMENT, None)]);
        assert!(matches!(
            crate::GbcFactory.create_split(&empty),
            Err(CreateSplitError::EmptyAssignments)
        ));
        let unknown =
            InputAssignments::new(vec![(AttachmentId::new("gbc.attachment.unknown"), None)]);
        assert!(matches!(
            crate::GbcFactory.create_split(&unknown),
            Err(CreateSplitError::SlotNotFound { .. })
        ));
    }

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
