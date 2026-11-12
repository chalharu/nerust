use std::rc::Rc;

use nerust_input_traits::{AttachmentId, ControllerProfile};

use super::model::app_state::PortDeviceSets;
pub use super::model::app_state::{
    BUILTIN_KEYBOARD_MEMBER, GAMEPAD_MEMBER_PREFIX, KEYBOARD_MEMBER_PREFIX,
};

/// Shareable members (keyboards) may sit in several ports at once.
pub fn member_is_shareable(member: &str) -> bool {
    member.starts_with(KEYBOARD_MEMBER_PREFIX)
}

/// Gamepad members: exclusive, one port at a time.
pub fn member_is_gamepad(member: &str) -> bool {
    member.starts_with(GAMEPAD_MEMBER_PREFIX)
}

/// Exclusive members (gamepads) appearing in more than one port.
/// Returns one entry per offending member: (member, ports).
pub fn exclusive_member_conflicts(ports: &PortDeviceSets) -> Vec<(String, Vec<String>)> {
    let mut by_member: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (port, members) in ports {
        for member in members {
            if !member_is_shareable(member) {
                by_member
                    .entry(member.as_str())
                    .or_default()
                    .push(port.as_str());
            }
        }
    }
    by_member
        .into_iter()
        .filter(|(_, ports)| ports.len() > 1)
        .map(|(member, ports)| {
            (
                member.to_string(),
                ports.into_iter().map(str::to_string).collect(),
            )
        })
        .collect()
}

/// Clear other occupied slots in the selected profile's multi-port set in place.
pub fn clear_multi_port_conflicts(
    slot: AttachmentId,
    profile: &dyn ControllerProfile,
    assignments: &mut [(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
) {
    if let Some(set) = profile
        .port_sets()
        .iter()
        .find(|set| set.ports.first() == Some(&slot))
    {
        for (other, controller) in assignments {
            if *other != slot && set.ports.contains(other) {
                *controller = None;
            }
        }
    }
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

    #[test]
    fn keyboards_share_ports_gamepads_do_not() {
        use super::{exclusive_member_conflicts, member_is_shareable};
        use crate::model::app_state::PortDeviceSets;
        assert!(member_is_shareable("kbd:builtin"));
        assert!(member_is_shareable("kbd:05ac:024f"));
        assert!(!member_is_shareable("pad:deadbeef"));
        let ports: PortDeviceSets = [
            (
                "p1".to_string(),
                vec!["kbd:builtin".to_string(), "pad:aa".to_string()],
            ),
            (
                "p2".to_string(),
                vec!["kbd:builtin".to_string(), "pad:aa".to_string()],
            ),
        ]
        .into_iter()
        .collect();
        // The shared keyboard is fine; the gamepad in both ports conflicts.
        let conflicts = exclusive_member_conflicts(&ports);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "pad:aa");
        assert_eq!(conflicts[0].1, vec!["p1".to_string(), "p2".to_string()]);
    }
}
