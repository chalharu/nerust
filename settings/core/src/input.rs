use std::{collections::HashSet, rc::Rc};

use nerust_input_traits::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, ControllerProfile, DeviceDescriptor,
    DeviceKindId, DigitalControlDescriptor, InputTopologyDescriptor, PortDescriptor, PortId,
    ProfileId, SlotInfo,
};

/// Map a controller profile + port group index to a device kind string.
pub fn device_kind(profile: &dyn ControllerProfile, group_index: usize) -> &'static str {
    profile.device_kind_for_group(group_index)
}

struct TopologyContext {
    ports: Vec<PortDescriptor>,
    seen_devices: HashSet<(ProfileId, usize)>,
    devices: Vec<DeviceDescriptor>,
}

impl TopologyContext {
    fn new() -> Self {
        Self {
            ports: Vec::new(),
            seen_devices: HashSet::new(),
            devices: Vec::new(),
        }
    }

    fn into_descriptor(self) -> InputTopologyDescriptor {
        if self.ports.is_empty() {
            InputTopologyDescriptor {
                ports: Vec::new(),
                devices: Vec::new(),
            }
        } else {
            InputTopologyDescriptor {
                ports: self.ports,
                devices: self.devices,
            }
        }
    }

    fn register_device(&mut self, profile: &dyn ControllerProfile, ctrl_id: ProfileId, gi: usize) {
        if !self.seen_devices.insert((ctrl_id, gi)) {
            return;
        }
        let dk = device_kind(profile, gi);
        let controls = profile.port_groups()[gi];
        self.devices.push(DeviceDescriptor {
            kind: DeviceKindId::new(dk),
            label: profile.label(),
            controls: controls
                .iter()
                .map(|ci| {
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: ci.id,
                        label: ci.label,
                        description: ci.label,
                    })
                })
                .collect(),
        });
    }

    fn register_port(
        &mut self,
        port: AttachmentId,
        profile: &dyn ControllerProfile,
        gi: usize,
        slot_label: &impl Fn(AttachmentId) -> &'static str,
    ) {
        let dk = device_kind(profile, gi);
        if self.ports.iter().any(|p: &PortDescriptor| p.id == port) {
            return;
        }
        let label = slot_label(port);
        self.ports.push(PortDescriptor {
            id: PortId::new(port.as_str()),
            label,
            attachments: vec![AttachmentSlotDescriptor {
                id: port,
                label,
                device: DeviceKindId::new(dk),
                supported_devices: vec![DeviceKindId::new(dk)],
            }],
        });
    }
}

/// Build an InputTopologyDescriptor from slot→controller assignments.
pub fn build_topology(
    assignments: &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
    slots: &[SlotInfo],
) -> InputTopologyDescriptor {
    let slot_label = |att: AttachmentId| -> &'static str {
        slots
            .iter()
            .find(|s| s.id == att)
            .map(|s| s.label)
            .unwrap_or("")
    };
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
                ctx.register_device(profile, profile.profile_id(), gi);
                ctx.register_port(port, profile, gi, &slot_label);
            }
        }
    }
    ctx.into_descriptor()
}

/// Clear other occupied slots in the same multi-port set.
pub fn clear_multi_port_conflicts(
    slot: AttachmentId,
    profile: &dyn ControllerProfile,
    assignments: &mut [(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
) {
    for ps in profile.port_sets() {
        if ps.ports.len() <= 1 || !ps.ports.contains(&slot) {
            continue;
        }
        for &port in ps.ports {
            if port != slot
                && let Some(other) = assignments.iter_mut().find(|(s, _)| *s == port)
            {
                other.1 = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use nerust_input_traits::{AttachmentId, ControllerProfile, PortSet, ProfileId};

    use super::{build_topology, clear_multi_port_conflicts, device_kind};

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
    fn device_kind_delegates_to_profile_method() {
        assert_eq!(device_kind(&SinglePortProfile, 0), "test.single");
        assert_eq!(device_kind(&MultiPortProfile, 0), "test.multi");
    }

    #[test]
    fn build_topology_empty_assignments() {
        let topology = build_topology(&[], &[]);
        assert!(topology.ports.is_empty());
        assert!(topology.devices.is_empty());
    }
}
