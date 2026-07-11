use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlInfo, ControlKind, Controller, ControllerProfile,
    DigitalControlId, OpenBusReadResult, Port, PortSet,
};

/// NES Standard Controller: full 8-button pad for a single port.
#[derive(Debug, Clone)]
pub struct StandardPad {
    pub(crate) cached: [u8; 2],
    pub(crate) result: [u8; 2],
    pub(crate) strobe: bool,
    /// Open bus mask for this port
    /// NES-001 & NES-004: 0x1F
    /// NES-101 & NES-039: 0x1B,0x1F (D2: OpenBus)
    open_bus_mask: u8,
}

impl StandardPad {
    pub fn new(open_bus_mask: u8) -> Self {
        Self {
            cached: [0; 2],
            result: [0; 2],
            strobe: false,
            open_bus_mask,
        }
    }
    /// Reset shift register for save state load.
    pub fn reset_runtime(&mut self) {
        self.result = [0; 2];
        self.strobe = false;
    }
}

impl Default for StandardPad {
    fn default() -> Self {
        Self::new(0x1F)
    }
}

impl Controller for StandardPad {
    fn sync_input(&mut self, state: &[u8]) {
        if let Some(s) = state.get(..2) {
            self.cached.copy_from_slice(s);
        }
    }
    fn read(&mut self, port: &dyn Port) -> OpenBusReadResult {
        let idx = port.index();
        let bit = if self.strobe {
            self.cached[idx] & 1
        } else {
            let b = self.result[idx] & 1;
            self.result[idx] = self.result[idx] >> 1 | 0x80;
            b
        };
        OpenBusReadResult::new(bit, self.open_bus_mask)
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
        ]
    }
}

#[derive(Debug)]
pub struct StandardPadProfile;

impl ControllerProfile for StandardPadProfile {
    fn id(&self) -> &'static str {
        "nes.standard_pad"
    }
    fn label(&self) -> &'static str {
        "NES Standard Controller"
    }
    fn port_sets(&self) -> &[PortSet] {
        &[
            PortSet {
                ports: &["player1"],
            },
            PortSet {
                ports: &["player2"],
            },
        ]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::*;
        static C: &[ControlInfo] = &[
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
        static G: &[&[ControlInfo]] = &[C];
        G
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}
