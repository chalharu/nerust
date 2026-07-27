use std::rc::Rc;

use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlInfo, ControlKind, Controller, ControllerProfile,
    DigitalControlId, OpenBusReadResult, Port, PortSet, ProfileId,
};

pub fn gbc_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    vec![Rc::new(StandardPadProfile) as Rc<dyn ControllerProfile>]
}

/// Game Boy / GBC controller: 8 buttons, read via joypad register $FF00.
#[derive(Debug, Clone)]
pub struct StandardPad {
    cached: u8,
}

impl StandardPad {
    pub fn new() -> Self {
        Self { cached: 0xFF }
    }
}

impl Default for StandardPad {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for StandardPad {
    fn sync_input(&mut self, state: &[u8]) {
        if let Some(s) = state.first() {
            self.cached = *s;
        }
    }

    fn read(&mut self, _port: &dyn Port) -> OpenBusReadResult {
        OpenBusReadResult::new(self.cached, 0x0F)
    }

    fn write(&mut self, _port: &dyn Port, _value: u8) {}

    fn field_map(&self, port: &dyn Port) -> Vec<(AttachmentId, DigitalControlId, usize)> {
        let attachment = port.as_attachment_id();
        vec![
            (attachment, DigitalControlId::new("gbo.control.a"), 3),
            (attachment, DigitalControlId::new("gbo.control.b"), 2),
            (attachment, DigitalControlId::new("gbo.control.select"), 5),
            (attachment, DigitalControlId::new("gbo.control.start"), 4),
            (attachment, DigitalControlId::new("gbo.control.down"), 7),
            (attachment, DigitalControlId::new("gbo.control.up"), 6),
            (attachment, DigitalControlId::new("gbo.control.left"), 9),
            (attachment, DigitalControlId::new("gbo.control.right"), 8),
        ]
    }
}

#[derive(Debug)]
pub struct StandardPadProfile;

impl ControllerProfile for StandardPadProfile {
    fn profile_id(&self) -> ProfileId {
        ProfileId::new("gbo.standard_pad")
    }

    fn label(&self) -> &'static str {
        "Game Boy Controller"
    }

    fn port_sets(&self) -> &[PortSet] {
        const P1: &[AttachmentId] = &[AttachmentId::new("gbc.attachment.player1")];
        const SETS: &[PortSet] = &[PortSet { ports: P1 }];
        SETS
    }

    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::Digital;
        const C: &[ControlInfo] = &[
            ControlInfo {
                id: DigitalControlId::new("gbo.control.a"),
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.b"),
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.select"),
                label: "Select",
                kind: Digital,
                abstract_key: Some(AbstractKey::Select),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.start"),
                label: "Start",
                kind: Digital,
                abstract_key: Some(AbstractKey::Start),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.up"),
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.down"),
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.left"),
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: DigitalControlId::new("gbo.control.right"),
                label: "Right",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadRight),
            },
        ];
        const G: &[&[ControlInfo]] = &[C];
        G
    }
}
