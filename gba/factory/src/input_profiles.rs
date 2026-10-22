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

impl crate::GbaFactory {
    pub(crate) fn create_split(
        &self,
        assignments: &InputAssignments,
    ) -> Result<
        nerust_input_traits::TypedInputResources<nerust_gba_core::input_types::GbaInputBuffer>,
        nerust_input_traits::CreateSplitError,
    > {
        use nerust_input_traits::{Controller as _, CreateSplitError};
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
        for entry in device.field_entries(&GBA_PORT) {
            let _ = map.insert((entry.attachment, entry.control), entry.field);
        }
        if map.is_empty() {
            return Err(CreateSplitError::EmptyAssignments);
        }
        Ok(nerust_input_traits::TypedInputResources::new(
            map,
            nerust_gba_core::input_types::GbaInputBuffer::default,
        ))
    }
}

/// Single-profile device registry: the canonical placement profile selects the device.
/// Unknown profiles cannot arrive through [`placement_plan`](InputAssignments::placement_plan);
/// the fallback names the actual offender for forward compatibility.
fn device_for_profile(
    profile: &Rc<dyn ControllerProfile>,
    _group: usize,
) -> Result<nerust_gba_device::StandardPad, nerust_input_traits::CreateSplitError> {
    use nerust_input_traits::CreateSplitError;
    if profile.profile_id() == ProfileId::new("gba.standard_pad") {
        Ok(nerust_gba_device::StandardPad::new())
    } else {
        Err(CreateSplitError::ControllerNotFound {
            controller: profile.profile_id(),
        })
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
    use nerust_input_traits::CreateSplitError;

    #[test]
    fn split_maps_ten_fields_and_rejects_bad_assignments() {
        let resources = crate::GbaFactory
            .create_split(&crate::GbaFactory.default_assignments())
            .unwrap();
        assert_eq!(resources.field_map().len(), 10);
        let empty = InputAssignments::new(vec![(GBA_ATTACHMENT, None)]);
        assert!(matches!(
            crate::GbaFactory.create_split(&empty),
            Err(CreateSplitError::EmptyAssignments)
        ));
        let unknown =
            InputAssignments::new(vec![(AttachmentId::new("gba.attachment.unknown"), None)]);
        assert!(matches!(
            crate::GbaFactory.create_split(&unknown),
            Err(CreateSplitError::SlotNotFound { .. })
        ));
    }

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
