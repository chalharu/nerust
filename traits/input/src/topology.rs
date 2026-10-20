//! Static input topology descriptors: ports, attachments, and devices.
//! Built once at init and shared read-only; init-only costs, never per-frame.

use super::ids::{
    AnalogControlId, AttachmentId, ControlId, DeviceKindId, DigitalControlId, PortId,
};

/// Static description of every port and device a system exposes.
/// Built once at init, shared read-only; the lookups are index-backed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTopologyDescriptor {
    ports: Vec<PortDescriptor>,
    devices: Vec<DeviceDescriptor>,
    attachment_index: std::collections::HashMap<AttachmentId, (usize, usize)>,
    device_index: std::collections::HashMap<DeviceKindId, usize>,
}

impl InputTopologyDescriptor {
    pub fn new(ports: Vec<PortDescriptor>, devices: Vec<DeviceDescriptor>) -> Self {
        let mut attachment_index = std::collections::HashMap::new();
        for (port_no, port) in ports.iter().enumerate() {
            for (slot_no, attachment) in port.attachments.iter().enumerate() {
                attachment_index.insert(attachment.id, (port_no, slot_no));
            }
        }
        let mut device_index = std::collections::HashMap::new();
        for (device_no, device) in devices.iter().enumerate() {
            device_index.insert(device.kind, device_no);
        }
        Self {
            ports,
            devices,
            attachment_index,
            device_index,
        }
    }

    pub fn ports(&self) -> &[PortDescriptor] {
        &self.ports
    }

    pub fn devices(&self) -> &[DeviceDescriptor] {
        &self.devices
    }

    /// Index-backed attachment lookup across all ports.
    pub fn attachment(&self, id: AttachmentId) -> Option<&AttachmentSlotDescriptor> {
        let (port_no, slot_no) = self.attachment_index.get(&id)?;
        Some(&self.ports[*port_no].attachments[*slot_no])
    }

    /// Index-backed device lookup by kind.
    pub fn device(&self, kind: DeviceKindId) -> Option<&DeviceDescriptor> {
        self.device_index.get(&kind).map(|i| &self.devices[*i])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDescriptor {
    id: PortId,
    label: &'static str,
    attachments: Vec<AttachmentSlotDescriptor>,
}

impl PortDescriptor {
    pub fn new(
        id: PortId,
        label: &'static str,
        attachments: Vec<AttachmentSlotDescriptor>,
    ) -> Self {
        Self {
            id,
            label,
            attachments,
        }
    }

    pub fn id(&self) -> PortId {
        self.id
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn attachments(&self) -> &[AttachmentSlotDescriptor] {
        &self.attachments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSlotDescriptor {
    id: AttachmentId,
    label: &'static str,
    device: DeviceKindId,
    supported_devices: Vec<DeviceKindId>,
}

impl AttachmentSlotDescriptor {
    pub fn new(
        id: AttachmentId,
        label: &'static str,
        device: DeviceKindId,
        supported_devices: Vec<DeviceKindId>,
    ) -> Self {
        Self {
            id,
            label,
            supported_devices,
            device,
        }
    }

    pub fn id(&self) -> AttachmentId {
        self.id
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn device(&self) -> DeviceKindId {
        self.device
    }

    pub fn supported_devices(&self) -> &[DeviceKindId] {
        &self.supported_devices
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    kind: DeviceKindId,
    label: &'static str,
    controls: Vec<ControlDescriptor>,
}

impl DeviceDescriptor {
    pub fn new(kind: DeviceKindId, label: &'static str, controls: Vec<ControlDescriptor>) -> Self {
        Self {
            kind,
            label,
            controls,
        }
    }

    pub fn kind(&self) -> DeviceKindId {
        self.kind
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn controls(&self) -> &[ControlDescriptor] {
        &self.controls
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlDescriptor {
    Digital(DigitalControlDescriptor),
    Analog(AnalogControlDescriptor),
}

impl ControlDescriptor {
    pub const fn id(&self) -> ControlId {
        match self {
            Self::Digital(control) => ControlId::digital(control.id),
            Self::Analog(control) => ControlId::analog(control.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalControlDescriptor {
    id: DigitalControlId,
    label: &'static str,
    description: &'static str,
}

impl DigitalControlDescriptor {
    pub const fn new(id: DigitalControlId, label: &'static str, description: &'static str) -> Self {
        Self {
            id,
            label,
            description,
        }
    }

    pub const fn id(&self) -> DigitalControlId {
        self.id
    }

    pub const fn label(&self) -> &'static str {
        self.label
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalogControlDescriptor {
    id: AnalogControlId,
    label: &'static str,
    description: &'static str,
}

impl AnalogControlDescriptor {
    pub const fn new(id: AnalogControlId, label: &'static str, description: &'static str) -> Self {
        Self {
            id,
            label,
            description,
        }
    }

    pub const fn id(&self) -> AnalogControlId {
        self.id
    }

    pub const fn label(&self) -> &'static str {
        self.label
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }
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
        let topology = InputTopologyDescriptor::new(
            vec![PortDescriptor::new(
                PortId::new("test.port1"),
                "Port 1",
                vec![AttachmentSlotDescriptor::new(
                    attachment,
                    "Player 1",
                    device,
                    vec![device],
                )],
            )],
            vec![DeviceDescriptor::new(
                device,
                "Gamepad",
                vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor::new(
                        DigitalControlId::new("test.a"),
                        "A",
                        "Primary face button",
                    )),
                    ControlDescriptor::Analog(AnalogControlDescriptor::new(
                        AnalogControlId::new("test.stick_x"),
                        "Stick X",
                        "Horizontal axis",
                    )),
                ],
            )],
        );

        assert_eq!(topology.attachment(attachment).unwrap().device(), device);
        let controls = &topology.device(device).unwrap().controls;
        assert_eq!(
            controls[0].id(),
            ControlId::digital(DigitalControlId::new("test.a"))
        );
        assert_eq!(
            controls[1].id(),
            ControlId::analog(AnalogControlId::new("test.stick_x"))
        );
    }

    #[test]
    fn missing_lookup_returns_none() {
        let topology = InputTopologyDescriptor::new(vec![], vec![]);
        assert!(topology.ports().is_empty());
        assert!(topology.devices().is_empty());
        assert!(topology.attachment(AttachmentId::new("nope")).is_none());
        assert!(topology.device(DeviceKindId::new("nope")).is_none());
    }

    #[test]
    fn descriptors_expose_labels_and_supported_devices() {
        let attachment = AttachmentId::new("test.pad1");
        let device = DeviceKindId::new("test.gamepad");
        let other = DeviceKindId::new("test.other");
        let slot =
            AttachmentSlotDescriptor::new(attachment, "Player 1", device, vec![device, other]);
        assert_eq!(slot.id(), attachment);
        assert_eq!(slot.label(), "Player 1");
        assert_eq!(slot.device(), device);
        assert_eq!(slot.supported_devices(), &[device, other]);
        let port = PortDescriptor::new(PortId::new("test.port1"), "Port 1", vec![slot]);
        assert_eq!(port.id(), PortId::new("test.port1"));
        assert_eq!(port.label(), "Port 1");
        assert_eq!(port.attachments().len(), 1);
        let dev = DeviceDescriptor::new(device, "Gamepad", vec![]);
        assert_eq!(dev.kind(), device);
        assert_eq!(dev.label(), "Gamepad");
        assert!(dev.controls().is_empty());
        let digital = DigitalControlDescriptor::new(DigitalControlId::new("test.a"), "A", "desc");
        assert_eq!(digital.id(), DigitalControlId::new("test.a"));
        assert_eq!(digital.label(), "A");
        assert_eq!(digital.description(), "desc");
        let analog = AnalogControlDescriptor::new(AnalogControlId::new("test.x"), "X", "axis");
        assert_eq!(analog.id(), AnalogControlId::new("test.x"));
        assert_eq!(analog.label(), "X");
        assert_eq!(analog.description(), "axis");
    }
}
