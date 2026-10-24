use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlId, ControlInfo, ControlKind, Controller, ControllerProfile,
    DigitalControlId, FieldEntry, OpenBusReadResult, Port, PortSet, ProfileId, StrobeTarget,
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

impl StrobeTarget for FamicomPadP1 {
    fn strobe(&mut self, value: u8) {
        let new_strobe = value & 1 == 1;
        if self.strobe && !new_strobe {
            self.result = self.cached_buttons;
        }
        self.strobe = new_strobe;
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
    fn field_entries(&self, port: &dyn Port) -> Vec<FieldEntry> {
        let attachment = port.attachment_id();
        let base = port.index() * 8;
        let digital = |id: &'static str, field: usize| {
            FieldEntry::new(
                attachment,
                ControlId::digital(DigitalControlId::new(id)),
                field,
            )
        };
        vec![
            digital("nes.control.a", base),
            digital("nes.control.b", base + 1),
            digital("nes.control.select", base + 2),
            digital("nes.control.start", base + 3),
            digital("nes.control.up", base + 4),
            digital("nes.control.down", base + 5),
            digital("nes.control.left", base + 6),
            digital("nes.control.right", base + 7),
            FieldEntry::new(
                AttachmentId::new("nes.attachment.player2"),
                ControlId::digital(DigitalControlId::new("famicom.microphone")),
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

impl StrobeTarget for FamicomPadP2 {
    fn strobe(&mut self, value: u8) {
        let new_strobe = value & 1 == 1;
        if self.strobe && !new_strobe {
            self.result = self.cached;
        }
        self.strobe = new_strobe;
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
    fn field_entries(&self, port: &dyn Port) -> Vec<FieldEntry> {
        let attachment = port.attachment_id();
        let base = port.index() * 8;
        let digital = |id: &'static str, field: usize| {
            FieldEntry::new(
                attachment,
                ControlId::digital(DigitalControlId::new(id)),
                field,
            )
        };
        vec![
            digital("nes.control.a", base),
            digital("nes.control.b", base + 1),
            digital("nes.control.up", base + 4),
            digital("nes.control.down", base + 5),
            digital("nes.control.left", base + 6),
            digital("nes.control.right", base + 7),
        ]
    }
}

#[derive(Debug)]
pub struct FamicomSetProfile;

impl ControllerProfile for FamicomSetProfile {
    fn profile_id(&self) -> ProfileId {
        ProfileId::new("nes.famicom")
    }
    fn label(&self) -> &'static str {
        "Famicom Controller Set"
    }
    fn port_sets(&self) -> &[PortSet] {
        const PORTS: &[AttachmentId] = &[
            AttachmentId::new("nes.attachment.player1"),
            AttachmentId::new("nes.attachment.player2"),
        ];
        const SETS: &[PortSet] = &[PortSet { ports: PORTS }];
        SETS
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::*;
        const P1: &[ControlInfo] = &[
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.a")),
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.b")),
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.select")),
                label: "Select",
                kind: Digital,
                abstract_key: Some(AbstractKey::Select),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.start")),
                label: "Start",
                kind: Digital,
                abstract_key: Some(AbstractKey::Start),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.up")),
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.down")),
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.left")),
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.right")),
                label: "Right",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadRight),
            },
        ];
        const P2: &[ControlInfo] = &[
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.a")),
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.b")),
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("famicom.microphone")),
                label: "Microphone",
                kind: Digital,
                // Third pad button: driven through pad-2 button 3, never
                // Select/Start (hardware-less no-ops).
                abstract_key: Some(AbstractKey::Button3),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.up")),
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.down")),
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.left")),
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: ControlId::digital(DigitalControlId::new("nes.control.right")),
                label: "Right",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadRight),
            },
        ];
        const G: &[&[ControlInfo]] = &[P1, P2];
        G
    }

    fn device_kind_for_group(&self, group_index: usize) -> nerust_input_traits::DeviceKindId {
        match group_index {
            1 => nerust_input_traits::DeviceKindId::new("nes.famicom_p2"),
            _ => nerust_input_traits::DeviceKindId::new("nes.famicom"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Controller, FamicomPadP1, FamicomPadP2, FamicomSetProfile, StrobeTarget as _};
    use nerust_input_traits::{ControllerProfile, ProfileId, SimplePort};

    fn p1() -> SimplePort {
        SimplePort::new(0, "nes.attachment.player1")
    }

    #[test]
    fn p1_reports_mic_bit_on_d2() {
        let mut pad = FamicomPadP1::default();
        pad.sync_input(&[0, 0, 1]);
        let port = p1();
        pad.strobe(1);
        assert_eq!(pad.read(&port).data & 4, 4);
        assert_eq!(pad.read(&port).mask, 7);
        pad.sync_input(&[0, 0, 0]);
        pad.strobe(1);
        assert_eq!(pad.read(&port).data & 4, 0);
    }

    #[test]
    fn p2_masks_select_start_and_shifts_six() {
        let mut pad = FamicomPadP2::default();
        pad.sync_input(&[0xFF, 0xFF]);
        let port = SimplePort::new(1, "nes.attachment.player2");
        pad.write(&port, 1);
        pad.write(&port, 0);
        let bits: Vec<u8> = (0..8).map(|_| pad.read(&port).data & 1).collect();
        assert_eq!(&bits[2..4], &[0, 0]);
        assert_eq!(pad.read(&port).mask, 0x1F);
        pad.reset_runtime();
        pad.write(&port, 1);
        assert_eq!(pad.read(&port).data & 1, 1);
    }

    #[test]
    fn profile_splits_mic_into_own_group() {
        let profile = FamicomSetProfile;
        assert_eq!(profile.profile_id(), ProfileId::new("nes.famicom"));
        assert_eq!(profile.port_sets().len(), 1);
        assert_eq!(profile.port_groups().len(), 2);
        assert_eq!(profile.device_kind_for_group(1).as_str(), "nes.famicom_p2");
        assert_eq!(profile.device_kind_for_group(0).as_str(), "nes.famicom");
        let pad1 = FamicomPadP1::default();
        assert_eq!(pad1.field_entries(&p1()).len(), 9);
    }
}
