//! Static input topology descriptors: ports, attachments, and devices.

use super::ids::{AnalogControlId, AttachmentId, DeviceKindId, DigitalControlId, PortId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    Digital(DigitalControlId),
    Analog(AnalogControlId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTopologyDescriptor {
    pub ports: Vec<PortDescriptor>,
    pub devices: Vec<DeviceDescriptor>,
}

impl InputTopologyDescriptor {
    pub fn attachment(&self, id: AttachmentId) -> Option<&AttachmentSlotDescriptor> {
        self.ports
            .iter()
            .flat_map(|port| port.attachments.iter())
            .find(|attachment| attachment.id == id)
    }

    pub fn device(&self, kind: DeviceKindId) -> Option<&DeviceDescriptor> {
        self.devices.iter().find(|device| device.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDescriptor {
    pub id: PortId,
    pub label: &'static str,
    pub attachments: Vec<AttachmentSlotDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSlotDescriptor {
    pub id: AttachmentId,
    pub label: &'static str,
    pub device: DeviceKindId,
    pub supported_devices: Vec<DeviceKindId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub kind: DeviceKindId,
    pub label: &'static str,
    pub controls: Vec<ControlDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlDescriptor {
    Digital(DigitalControlDescriptor),
    Analog(AnalogControlDescriptor),
}

impl ControlDescriptor {
    pub const fn id(&self) -> ControlId {
        match self {
            Self::Digital(control) => ControlId::Digital(control.id),
            Self::Analog(control) => ControlId::Analog(control.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalControlDescriptor {
    pub id: DigitalControlId,
    pub label: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalogControlDescriptor {
    pub id: AnalogControlId,
    pub label: &'static str,
    pub description: &'static str,
}

#[cfg(test)]
mod tests {
    use super::{
        AnalogControlDescriptor, AnalogControlId, AttachmentId, AttachmentSlotDescriptor,
        ControlDescriptor, ControlId, DeviceDescriptor, DeviceKindId, DigitalControlDescriptor,
        DigitalControlId, InputTopologyDescriptor, PortDescriptor, PortId,
    };

    #[test]
    fn topology_tracks_ports_attachments_and_devices() {
        let attachment = AttachmentId::new("test.pad1");
        let device = DeviceKindId::new("test.gamepad");
        let topology = InputTopologyDescriptor {
            ports: vec![PortDescriptor {
                id: PortId::new("test.port1"),
                label: "Port 1",
                attachments: vec![AttachmentSlotDescriptor {
                    id: attachment,
                    label: "Player 1",
                    device,
                    supported_devices: vec![device],
                }],
            }],
            devices: vec![DeviceDescriptor {
                kind: device,
                label: "Gamepad",
                controls: vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: DigitalControlId::new("test.a"),
                        label: "A",
                        description: "Primary face button",
                    }),
                    ControlDescriptor::Analog(AnalogControlDescriptor {
                        id: AnalogControlId::new("test.stick_x"),
                        label: "Stick X",
                        description: "Horizontal axis",
                    }),
                ],
            }],
        };

        assert_eq!(topology.attachment(attachment).unwrap().device, device);
        let controls = &topology.device(device).unwrap().controls;
        assert_eq!(
            controls[0].id(),
            ControlId::Digital(DigitalControlId::new("test.a"))
        );
        assert_eq!(
            controls[1].id(),
            ControlId::Analog(AnalogControlId::new("test.stick_x"))
        );
    }
}
