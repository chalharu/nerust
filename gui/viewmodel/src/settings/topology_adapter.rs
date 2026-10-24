//! Topology adapter: project the input descriptor from slot assignments. Moved from `settings-core`:
//! settings stores persisted string pairs only; profile-to-descriptor projection lives here, next to the view models.

use std::{collections::HashSet, rc::Rc};

use nerust_input_traits::{
    AnalogControlDescriptor, AttachmentId, AttachmentSlotDescriptor, ControlDescriptor,
    ControllerProfile, DeviceDescriptor, DeviceKindId, DigitalControlDescriptor,
    InputTopologyDescriptor, PortDescriptor, PortId, SlotInfo,
};

struct TopologyContext {
    ports: Vec<PortDescriptor>,
    seen_ports: std::collections::HashSet<PortId>,
    seen_devices: HashSet<DeviceKindId>,
    devices: Vec<DeviceDescriptor>,
}

impl TopologyContext {
    fn new() -> Self {
        Self {
            ports: Vec::new(),
            seen_ports: std::collections::HashSet::new(),
            seen_devices: HashSet::new(),
            devices: Vec::new(),
        }
    }

    fn into_descriptor(self) -> InputTopologyDescriptor {
        if self.ports.is_empty() {
            InputTopologyDescriptor::new(Vec::new(), Vec::new())
        } else {
            InputTopologyDescriptor::new(self.ports, self.devices)
        }
    }

    fn register_device(&mut self, profile: &dyn ControllerProfile, gi: usize) {
        let dk = profile.device_kind_for_group(gi);
        if !self.seen_devices.insert(dk) {
            return;
        }
        let controls = profile.port_groups()[gi];
        self.devices.push(DeviceDescriptor::new(
            dk,
            profile.label(),
            controls
                .iter()
                .filter_map(|ci| {
                    if let Some(id) = ci.id.as_digital() {
                        Some(ControlDescriptor::Digital(DigitalControlDescriptor::new(
                            id, ci.label, ci.label,
                        )))
                    } else {
                        ci.id.as_analog().map(|id| {
                            ControlDescriptor::Analog(AnalogControlDescriptor::new(
                                id, ci.label, ci.label,
                            ))
                        })
                    }
                })
                .collect(),
        ));
    }

    fn register_port(
        &mut self,
        port: AttachmentId,
        profile: &dyn ControllerProfile,
        gi: usize,
        slot_label: &impl Fn(AttachmentId) -> &'static str,
    ) {
        let dk = profile.device_kind_for_group(gi);
        let port_id = PortId::new(port.as_str());
        if !self.seen_ports.insert(port_id) {
            return;
        }
        let label = slot_label(port);
        self.ports.push(PortDescriptor::new(
            port_id,
            label,
            vec![AttachmentSlotDescriptor::new(port, label, dk, vec![dk])],
        ));
    }
}

/// Build an InputTopologyDescriptor from slot→controller assignments.
pub fn build_topology(
    assignments: &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
    slots: &[SlotInfo],
) -> InputTopologyDescriptor {
    // Indexed once: label lookup and port dedup are O(1) per assignment.
    let labels: std::collections::HashMap<AttachmentId, &'static str> =
        slots.iter().map(|s| (s.id, s.label)).collect();
    let slot_label =
        |att: AttachmentId| -> &'static str { labels.get(&att).copied().unwrap_or("") };
    let mut ctx = TopologyContext::new();

    for (slot_att, ctrl_opt) in assignments {
        let profile = match ctrl_opt {
            Some(p) => p.as_ref(),
            None => continue,
        };
        for ps in profile.port_sets() {
            if !ps.ports.contains(slot_att) {
                continue;
            }
            for (gi, &port) in ps.ports.iter().enumerate() {
                ctx.register_device(profile, gi);
                ctx.register_port(port, profile, gi, &slot_label);
            }
        }
    }
    ctx.into_descriptor()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use nerust_input_traits::{AttachmentId, ControllerProfile, PortSet, ProfileId};

    use super::build_topology;

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

    const P1: AttachmentId = AttachmentId::new("p1");

    #[test]
    fn build_topology_empty_assignments() {
        let topology = build_topology(&[], &[]);
        assert!(topology.ports().is_empty());
        assert!(topology.devices().is_empty());
    }

    #[test]
    fn build_topology_maps_single_assignment() {
        let assignments = vec![(
            P1,
            Some(Rc::new(SinglePortProfile) as Rc<dyn ControllerProfile>),
        )];
        let slots = vec![nerust_input_traits::SlotInfo {
            id: P1,
            label: "P1",
        }];
        let topology = build_topology(&assignments, &slots);
        assert_eq!(topology.ports().len(), 1);
        assert_eq!(topology.devices().len(), 1);
    }

    #[test]
    fn unassigned_slots_and_unknown_labels_are_skipped() {
        let assignments = vec![(P1, None)];
        let slots = vec![nerust_input_traits::SlotInfo {
            id: P1,
            label: "P1",
        }];
        assert!(build_topology(&assignments, &slots).ports().is_empty());
        let assignments = vec![(
            P1,
            Some(Rc::new(SinglePortProfile) as Rc<dyn ControllerProfile>),
        )];
        let topology = build_topology(&assignments, &[]);
        assert_eq!(topology.ports().len(), 1);
        assert_eq!(topology.ports()[0].label(), "");
    }

    #[test]
    fn repeated_assignments_share_one_device() {
        let assignments = vec![
            (
                P1,
                Some(Rc::new(SinglePortProfile) as Rc<dyn ControllerProfile>),
            ),
            (
                P1,
                Some(Rc::new(SinglePortProfile) as Rc<dyn ControllerProfile>),
            ),
        ];
        let slots = vec![nerust_input_traits::SlotInfo {
            id: P1,
            label: "P1",
        }];
        let topology = build_topology(&assignments, &slots);
        assert_eq!(topology.ports().len(), 1);
        assert_eq!(topology.devices().len(), 1);
    }
}
