use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlInfo, ControlKind, Controller, ControllerProfile,
    DigitalControlId, OpenBusReadResult, Port, PortSet,
};

/// Famicom controller on port 1: 8 buttons + microphone on D2 ($4016).
#[derive(Debug, Clone)]
pub struct FamicomPadP1 {
    cached_buttons: u8,
    cached_mic: u8,
    result: u8,
    strobe: bool,
}

impl FamicomPadP1 {
    pub fn new() -> Self {
        Self {
            cached_buttons: 0,
            cached_mic: 0,
            result: 0,
            strobe: false,
        }
    }
    pub fn reset_runtime(&mut self) {
        self.result = 0;
        self.strobe = false;
    }
}

impl Default for FamicomPadP1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for FamicomPadP1 {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 3 {
            self.cached_buttons = state[0];
            self.cached_mic = state[2];
        }
    }
    fn read(&mut self, _port: &dyn Port) -> OpenBusReadResult {
        let bit = if self.strobe {
            self.cached_buttons & 1
        } else {
            let b = self.result & 1;
            self.result = self.result >> 1 | 0x80;
            b
        };
        let mic = if self.cached_mic != 0 { 4 } else { 0 };
        OpenBusReadResult::new(bit | mic, 7)
    }
    fn write(&mut self, _port: &dyn Port, value: u8) {
        let new_strobe = value & 1 == 1;
        if self.strobe && !new_strobe {
            self.result = self.cached_buttons;
        }
        self.strobe = new_strobe;
    }
    fn field_map(&self, port: &dyn Port) -> Vec<(AttachmentId, DigitalControlId, usize)> {
        let attachment = port.as_attachment_id();
        let base = port.index() * 8;
        vec![
            (attachment, DigitalControlId::new("nes.control.a"), base),
            (attachment, DigitalControlId::new("nes.control.b"), base + 1),
            (
                attachment,
                DigitalControlId::new("nes.control.select"),
                base + 2,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.start"),
                base + 3,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.up"),
                base + 4,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.down"),
                base + 5,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.left"),
                base + 6,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.right"),
                base + 7,
            ),
            (
                AttachmentId::new("nes.attachment.player2"),
                DigitalControlId::new("famicom.microphone"),
                16,
            ),
        ]
    }
}

/// Famicom controller on port 2: 6 buttons (Select/Start always 0).
#[derive(Debug, Clone)]
pub struct FamicomPadP2 {
    cached: u8,
    result: u8,
    strobe: bool,
}

impl FamicomPadP2 {
    pub fn new() -> Self {
        Self {
            cached: 0,
            result: 0,
            strobe: false,
        }
    }
    pub fn reset_runtime(&mut self) {
        self.result = 0;
        self.strobe = false;
    }
}

impl Default for FamicomPadP2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for FamicomPadP2 {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 2 {
            self.cached = state[1] & 0b11110011;
        }
    }
    fn read(&mut self, _port: &dyn Port) -> OpenBusReadResult {
        let bit = if self.strobe {
            self.cached & 1
        } else {
            let b = self.result & 1;
            self.result = self.result >> 1 | 0x80;
            b
        };
        OpenBusReadResult::new(bit, 0x1F)
    }
    fn write(&mut self, _port: &dyn Port, value: u8) {
        let new_strobe = value & 1 == 1;
        if self.strobe && !new_strobe {
            self.result = self.cached;
        }
        self.strobe = new_strobe;
    }
    fn field_map(&self, port: &dyn Port) -> Vec<(AttachmentId, DigitalControlId, usize)> {
        let attachment = port.as_attachment_id();
        let base = port.index() * 8;
        vec![
            (attachment, DigitalControlId::new("nes.control.a"), base),
            (attachment, DigitalControlId::new("nes.control.b"), base + 1),
            (
                attachment,
                DigitalControlId::new("nes.control.up"),
                base + 4,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.down"),
                base + 5,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.left"),
                base + 6,
            ),
            (
                attachment,
                DigitalControlId::new("nes.control.right"),
                base + 7,
            ),
        ]
    }
}

#[derive(Debug)]
pub struct FamicomSetProfile;

impl ControllerProfile for FamicomSetProfile {
    fn id(&self) -> &'static str {
        "nes.famicom"
    }
    fn label(&self) -> &'static str {
        "Famicom Controller Set"
    }
    fn port_sets(&self) -> &[PortSet] {
        &[PortSet {
            ports: &["player1", "player2"],
        }]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::*;
        static P1: &[ControlInfo] = &[
            ControlInfo {
                id: "a",
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: "b",
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: "select",
                label: "Select",
                kind: Digital,
                abstract_key: Some(AbstractKey::Select),
            },
            ControlInfo {
                id: "start",
                label: "Start",
                kind: Digital,
                abstract_key: Some(AbstractKey::Start),
            },
            ControlInfo {
                id: "up",
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: "down",
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: "left",
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: "right",
                label: "Right",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadRight),
            },
        ];
        static P2: &[ControlInfo] = &[
            ControlInfo {
                id: "a",
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: "b",
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: "microphone",
                label: "Microphone",
                kind: Digital,
                abstract_key: None,
            },
            ControlInfo {
                id: "up",
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: "down",
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: "left",
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: "right",
                label: "Right",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadRight),
            },
        ];
        static G: &[&[ControlInfo]] = &[P1, P2];
        G
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}
