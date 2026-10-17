use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlInfo, ControlKind, Controller, ControllerProfile,
    DigitalControlId, OpenBusReadResult, Port, PortSet, ProfileId,
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
    fn profile_id(&self) -> ProfileId {
        ProfileId::new("nes.standard_pad")
    }
    fn label(&self) -> &'static str {
        "NES Standard Controller"
    }
    fn port_sets(&self) -> &[PortSet] {
        const P1: &[AttachmentId] = &[AttachmentId::new("nes.attachment.player1")];
        const P2: &[AttachmentId] = &[AttachmentId::new("nes.attachment.player2")];
        const SETS: &[PortSet] = &[PortSet { ports: P1 }, PortSet { ports: P2 }];
        SETS
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::*;
        const C: &[ControlInfo] = &[
            ControlInfo {
                id: DigitalControlId::new("nes.control.a"),
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.b"),
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.select"),
                label: "Select",
                kind: Digital,
                abstract_key: Some(AbstractKey::Select),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.start"),
                label: "Start",
                kind: Digital,
                abstract_key: Some(AbstractKey::Start),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.up"),
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.down"),
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.left"),
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: DigitalControlId::new("nes.control.right"),
                label: "Right",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadRight),
            },
        ];
        const G: &[&[ControlInfo]] = &[C];
        G
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_input_traits::SimplePort;

    fn player1() -> SimplePort {
        SimplePort::new(0, "nes.attachment.player1")
    }

    #[test]
    fn strobe_high_holds_first_button() {
        let mut pad = StandardPad::new(0x1F);
        pad.sync_input(&[0b1010_0101, 0]);
        let port = player1();
        pad.write(&port, 1);
        // A (bit 0) reads back while strobe stays high.
        assert_eq!(pad.read(&port).data & 1, 1);
        assert_eq!(pad.read(&port).data & 1, 1);
        assert_eq!(pad.read(&port).mask, 0x1F);
    }

    #[test]
    fn strobe_fall_shifts_out_buttons_then_ones() {
        let mut pad = StandardPad::new(0x1F);
        pad.sync_input(&[0b1010_0101, 0]);
        let port = player1();
        pad.write(&port, 1);
        pad.write(&port, 0);
        // A, B, Select, Start, Up, Down, Left, Right, then open 1s.
        let bits: Vec<u8> = (0..10).map(|_| pad.read(&port).data & 1).collect();
        assert_eq!(&bits[..8], &[1, 0, 1, 0, 0, 1, 0, 1]);
        assert_eq!(&bits[8..], &[1, 1]);
    }

    #[test]
    fn second_port_reads_second_byte() {
        let mut pad = StandardPad::new(0x1F);
        pad.sync_input(&[0, 0b0000_0010]);
        let port = SimplePort::new(1, "nes.attachment.player2");
        pad.write(&port, 1);
        pad.write(&port, 0);
        let bits: Vec<u8> = (0..8).map(|_| pad.read(&port).data & 1).collect();
        assert_eq!(&bits, &[0, 1, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn reset_runtime_clears_shift_register() {
        let mut pad = StandardPad::new(0x1F);
        pad.sync_input(&[0xFF, 0xFF]);
        let port = player1();
        pad.write(&port, 1);
        pad.write(&port, 0);
        pad.read(&port);
        pad.reset_runtime();
        // Strobe latches fresh input again after reset.
        pad.write(&port, 1);
        assert_eq!(pad.read(&port).data & 1, 1);
    }

    #[test]
    fn field_map_covers_eight_buttons_in_order() {
        let pad = StandardPad::default();
        let map = pad.field_map(&player1());
        let ids: Vec<&str> = map.iter().map(|(_, id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "nes.control.a",
                "nes.control.b",
                "nes.control.select",
                "nes.control.start",
                "nes.control.up",
                "nes.control.down",
                "nes.control.left",
                "nes.control.right",
            ]
        );
        let offsets: Vec<usize> = map.iter().map(|(_, _, bit)| *bit).collect();
        assert_eq!(offsets, [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn profile_describes_two_ports_and_eight_controls() {
        let profile = StandardPadProfile;
        assert_eq!(profile.profile_id().as_str(), "nes.standard_pad");
        assert_eq!(profile.label(), "NES Standard Controller");
        assert_eq!(profile.port_sets().len(), 2);
        assert_eq!(profile.port_groups().len(), 1);
        assert_eq!(profile.port_groups()[0].len(), 8);
    }
}
