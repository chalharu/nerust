use std::rc::Rc;

use nerust_input_traits::{AttachmentId, ControllerProfile, InputAssignments};

/// Clear other occupied slots in the same multi-port set; owned by [`InputAssignments`].
pub fn clear_multi_port_conflicts(
    slot: AttachmentId,
    profile: &dyn ControllerProfile,
    assignments: &mut [(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
) {
    let mut owned = InputAssignments::new(assignments.to_vec());
    owned.clear_multi_port_conflicts(slot, profile);
    assignments.clone_from_slice(owned.slots());
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use nerust_input_traits::{AttachmentId, ControllerProfile, PortSet, ProfileId};

    use super::clear_multi_port_conflicts;

    #[derive(Debug)]
    struct SinglePortProfile;
    impl ControllerProfile for SinglePortProfile {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.single")
        }
        fn label(&self) -> &'static str {
            "Single"
        }
        fn port_sets(&self) -> &[PortSet] {
            static PORTS: [PortSet; 1] = [PortSet { ports: &[P1] }];
            &PORTS
        }
        fn port_groups(&self) -> &[&[nerust_input_traits::ControlInfo]] {
            static EMPTY: [nerust_input_traits::ControlInfo; 0] = [];
            static GROUPS: [&[nerust_input_traits::ControlInfo]; 1] = [&EMPTY];
            &GROUPS
        }
    }

    #[derive(Debug)]
    struct MultiPortProfile;
    impl ControllerProfile for MultiPortProfile {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.multi")
        }
        fn label(&self) -> &'static str {
            "Multi"
        }
        fn port_sets(&self) -> &[PortSet] {
            static PORTS: [PortSet; 1] = [PortSet { ports: &[P1, P2] }];
            &PORTS
        }
        fn port_groups(&self) -> &[&[nerust_input_traits::ControlInfo]] {
            static EMPTY: [nerust_input_traits::ControlInfo; 0] = [];
            static GROUPS: [&[nerust_input_traits::ControlInfo]; 1] = [&EMPTY];
            &GROUPS
        }
    }

    const P1: AttachmentId = AttachmentId::new("p1");
    const P2: AttachmentId = AttachmentId::new("p2");
    const OTHER: AttachmentId = AttachmentId::new("other");

    #[test]
    fn clear_multi_port_does_nothing_for_single_port() {
        let mut assignments = vec![(
            P1,
            Some(Rc::new(SinglePortProfile) as Rc<dyn ControllerProfile>),
        )];
        clear_multi_port_conflicts(P1, &SinglePortProfile, &mut assignments);
        assert!(assignments[0].1.is_some());
    }

    #[test]
    fn clear_multi_port_clears_other_ports() {
        let p = Rc::new(MultiPortProfile) as Rc<dyn ControllerProfile>;
        let mut assignments = vec![(P1, Some(Rc::clone(&p))), (P2, Some(Rc::clone(&p)))];
        clear_multi_port_conflicts(P1, &MultiPortProfile, &mut assignments);
        assert!(assignments[0].1.is_some());
        assert!(assignments[1].1.is_none());
    }

    #[test]
    fn clear_multi_port_does_not_clear_unrelated() {
        let multi = Rc::new(MultiPortProfile) as Rc<dyn ControllerProfile>;
        let single = Rc::new(SinglePortProfile) as Rc<dyn ControllerProfile>;
        let mut assignments = vec![
            (OTHER, Some(Rc::clone(&single))),
            (P1, Some(Rc::clone(&multi))),
            (P2, Some(Rc::clone(&multi))),
        ];
        clear_multi_port_conflicts(P1, &MultiPortProfile, &mut assignments);
        assert!(assignments[0].1.is_some(), "unrelated slot untouched");
        assert!(assignments[1].1.is_some());
        assert!(assignments[2].1.is_none());
    }
}
