use std::{collections::HashSet, rc::Rc};

use nerust_input_traits::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, ControllerProfile, DeviceDescriptor,
    DeviceKindId, DigitalControlDescriptor, InputTopologyDescriptor, PortDescriptor, PortId,
    SlotInfo,
};

/// Map a controller profile + port group index to a device kind string.
pub fn device_kind(profile: &dyn ControllerProfile, group_index: usize) -> &'static str {
    profile.device_kind_for_group(group_index)
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
    let mut ports = Vec::new();
    let mut seen_devices = HashSet::<(&str, usize)>::new();
    let mut devices = Vec::new();

    for (slot_att, ctrl_opt) in assignments {
        let profile = match ctrl_opt {
            Some(p) => p.as_ref(),
            None => continue,
        };
        let ctrl_id = profile.profile_id().as_str();
        for ps in profile.port_sets() {
            if ps.ports.contains(slot_att) {
                for (gi, &port) in ps.ports.iter().enumerate() {
                    let dk = device_kind(profile, gi);
                    if seen_devices.insert((ctrl_id, gi)) {
                        let controls = profile.port_groups()[gi];
                        devices.push(DeviceDescriptor {
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
                    if !ports.iter().any(|p: &PortDescriptor| p.id == port) {
                        let label = slot_label(port);
                        ports.push(PortDescriptor {
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
            }
        }
    }
    if ports.is_empty() {
        InputTopologyDescriptor {
            ports: Vec::new(),
            devices: Vec::new(),
        }
    } else {
        InputTopologyDescriptor { ports, devices }
    }
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
