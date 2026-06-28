use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Unexpected, Visitor},
};

/// システム識別子。CoreFactory impl のみが生成する。
/// 比較は `Eq` 経由のみ。生文字列の取り出しは不可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemId(&'static str);

impl SystemId {
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }
}

impl fmt::Display for SystemId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for SystemId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for SystemId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(SystemIdVisitor)
    }
}

struct SystemIdVisitor;

impl<'de> Visitor<'de> for SystemIdVisitor {
    type Value = SystemId;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a system identifier string")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<SystemId, E> {
        Ok(SystemId(match v {
            "Nes" | "nes" => "nes",
            "Snes" | "snes" => "snes",
            "Ps1" | "ps1" => "ps1",
            "MegaDrive" | "megadrive" => "megadrive",
            other => return Err(E::invalid_value(Unexpected::Str(other), &self)),
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortId(&'static str);

impl PortId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachmentId(&'static str);

impl AttachmentId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceKindId(&'static str);

impl DeviceKindId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DigitalControlId(&'static str);

impl DigitalControlId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalogControlId(&'static str);

impl AnalogControlId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitalInputState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitalInputEvent {
    pub attachment: AttachmentId,
    pub control: DigitalControlId,
    pub state: DigitalInputState,
}

impl DigitalInputEvent {
    pub const fn new(
        attachment: AttachmentId,
        control: DigitalControlId,
        state: DigitalInputState,
    ) -> Self {
        Self {
            attachment,
            control,
            state,
        }
    }

    pub const fn pressed(attachment: AttachmentId, control: DigitalControlId) -> Self {
        Self::new(attachment, control, DigitalInputState::Pressed)
    }

    pub const fn released(attachment: AttachmentId, control: DigitalControlId) -> Self {
        Self::new(attachment, control, DigitalInputState::Released)
    }

    pub const fn is_pressed(self) -> bool {
        matches!(self.state, DigitalInputState::Pressed)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnalogControlDescriptor, AnalogControlId, AttachmentId, AttachmentSlotDescriptor,
        ControlDescriptor, ControlId, DeviceDescriptor, DeviceKindId, DigitalControlDescriptor,
        DigitalControlId, DigitalInputEvent, DigitalInputState, InputTopologyDescriptor,
        PortDescriptor, PortId,
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

    #[test]
    fn digital_input_event_helpers_preserve_state() {
        let attachment = AttachmentId::new("test.pad1");
        let control = DigitalControlId::new("test.a");

        assert!(DigitalInputEvent::pressed(attachment, control).is_pressed());
        assert_eq!(
            DigitalInputEvent::released(attachment, control).state,
            DigitalInputState::Released
        );
    }
}
