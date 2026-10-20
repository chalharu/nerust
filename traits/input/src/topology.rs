//! Static input topology descriptors: ports, attachments, and devices.
//!
//! These descriptors are built once during factory/session initialization
//! and then shared read-only. The `Vec` storage and the linear
//! [`InputTopologyDescriptor::attachment`]/[`InputTopologyDescriptor::device`]
//! scans are init-only costs; do not query them per frame on the hot path.
//! Cloning a topology is cheap enough for init-time sharing (small vectors
//! of descriptors) but clone-once-and-share (`Arc`) rather than cloning
//! per frame.

use super::ids::{AnalogControlId, AttachmentId, DeviceKindId, DigitalControlId, PortId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    Digital(DigitalControlId),
    Analog(AnalogControlId),
}

/// Static description of every port and device a system exposes.
///
/// Built once at init time and shared read-only afterwards; the linear
/// lookup helpers below are init-only conveniences, not per-frame queries.
/// Construct with [`InputTopologyDescriptor::new`]; read with
/// [`InputTopologyDescriptor::ports`]/[`InputTopologyDescriptor::devices`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTopologyDescriptor {
    ports: Vec<PortDescriptor>,
    devices: Vec<DeviceDescriptor>,
}

impl InputTopologyDescriptor {
    pub fn new(ports: Vec<PortDescriptor>, devices: Vec<DeviceDescriptor>) -> Self {
        Self { ports, devices }
    }

    pub fn ports(&self) -> &[PortDescriptor] {
        &self.ports
    }

    pub fn devices(&self) -> &[DeviceDescriptor] {
        &self.devices
    }

    /// Init-only linear scan for an attachment across all ports.
    pub fn attachment(&self, id: AttachmentId) -> Option<&AttachmentSlotDescriptor> {
        self.ports
            .iter()
            .flat_map(|port| port.attachments.iter())
            .find(|attachment| attachment.id == id)
    }

    /// Init-only linear scan for a device by kind.
    pub fn device(&self, kind: DeviceKindId) -> Option<&DeviceDescriptor> {
        self.devices.iter().find(|device| device.kind == kind)
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
            Self::Digital(control) => ControlId::Digital(control.id),
            Self::Analog(control) => ControlId::Analog(control.id),
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
            ControlId::Digital(DigitalControlId::new("test.a"))
        );
        assert_eq!(
            controls[1].id(),
            ControlId::Analog(AnalogControlId::new("test.stick_x"))
        );
    }
}
