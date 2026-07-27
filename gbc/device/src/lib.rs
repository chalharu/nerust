use std::rc::Rc;

use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlInfo, ControlKind, Controller, ControllerProfile,
    DigitalControlId, OpenBusReadResult, Port, PortSet, ProfileId,
};

pub fn gbc_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    vec![Rc::new(StandardPadProfile) as Rc<dyn ControllerProfile>]
}

/// GBC joypad bit assignments for the 1-byte input buffer.
///
/// Field indices 0-7 correspond to the bit positions used by
/// `GbcInputBuffer::set()` to map each button to a joypad register bit.
const FIELD_A: usize = 0;
const FIELD_B: usize = 1;
const FIELD_SELECT: usize = 2;
const FIELD_START: usize = 3;
const FIELD_RIGHT: usize = 4;
const FIELD_LEFT: usize = 5;
const FIELD_UP: usize = 6;
const FIELD_DOWN: usize = 7;

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
            (attachment, DigitalControlId::new("gbc.control.a"), FIELD_A),
            (attachment, DigitalControlId::new("gbc.control.b"), FIELD_B),
            (attachment, DigitalControlId::new("gbc.control.select"), FIELD_SELECT),
            (attachment, DigitalControlId::new("gbc.control.start"), FIELD_START),
            (attachment, DigitalControlId::new("gbc.control.right"), FIELD_RIGHT),
            (attachment, DigitalControlId::new("gbc.control.left"), FIELD_LEFT),
            (attachment, DigitalControlId::new("gbc.control.up"), FIELD_UP),
            (attachment, DigitalControlId::new("gbc.control.down"), FIELD_DOWN),
        ]
    }
}

#[derive(Debug)]
pub struct StandardPadProfile;

impl ControllerProfile for StandardPadProfile {
    fn profile_id(&self) -> ProfileId {
        ProfileId::new("gbc.standard_pad")
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
                id: DigitalControlId::new("gbc.control.a"),
                label: "A",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button1),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.b"),
                label: "B",
                kind: Digital,
                abstract_key: Some(AbstractKey::Button2),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.select"),
                label: "Select",
                kind: Digital,
                abstract_key: Some(AbstractKey::Select),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.start"),
                label: "Start",
                kind: Digital,
                abstract_key: Some(AbstractKey::Start),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.up"),
                label: "Up",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadUp),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.down"),
                label: "Down",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadDown),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.left"),
                label: "Left",
                kind: Digital,
                abstract_key: Some(AbstractKey::DpadLeft),
            },
            ControlInfo {
                id: DigitalControlId::new("gbc.control.right"),
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

    #[test]
    fn default_pad_releases_all_buttons() {
        let pad = StandardPad::default();
        assert_eq!(pad.cached, 0xFF);
    }

    #[test]
    fn sync_input_stores_byte() {
        let mut pad = StandardPad::default();
        pad.sync_input(&[0x00]);
        assert_eq!(pad.cached, 0x00);
    }

    #[test]
    fn sync_input_ignores_empty_state() {
        let mut pad = StandardPad::default();
        pad.sync_input(&[]);
        assert_eq!(pad.cached, 0xFF);
    }

    #[test]
    fn field_map_has_eight_entries() {
        let pad = StandardPad::default();
        let map = pad.field_map(&nerust_input_traits::SimplePort::new(0, "test"));
        assert_eq!(map.len(), 8);
    }

    #[test]
    fn field_map_indices_are_unique_and_in_range() {
        let pad = StandardPad::default();
        let map = pad.field_map(&nerust_input_traits::SimplePort::new(0, "test"));
        let indices: Vec<usize> = map.iter().map(|(_, _, idx)| *idx).collect();
        let mut sorted = indices.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(indices.len(), sorted.len());
        for i in sorted {
            assert!(i < 8, "field index {i} exceeds buffer byte");
        }
    }

    #[test]
    fn profile_id_matches_expected() {
        let profile = StandardPadProfile;
        assert_eq!(profile.profile_id().as_str(), "gbc.standard_pad");
    }

    #[test]
    fn profile_has_single_port_set() {
        let profile = StandardPadProfile;
        assert_eq!(profile.port_sets().len(), 1);
        assert_eq!(profile.port_sets()[0].ports.len(), 1);
    }
}
