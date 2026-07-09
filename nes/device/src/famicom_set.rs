use nerust_input_traits::{AbstractKey, ControlInfo, ControlKind, ControllerProfile, PortSet};
use nerust_nes_core::{OpenBusReadResult, controller::Controller};

use crate::pad_common;

/// Famicom Controller Set: P1=8 buttons, P2=6 buttons + microphone.
#[derive(Debug, Clone)]
pub struct FamicomSet {
    pub(crate) cached: [u8; 3],
    pub(crate) index: [u8; 2],
    pub(crate) strobe: bool,
}

impl FamicomSet {
    pub fn new() -> Self {
        Self {
            cached: [0; 3],
            index: [0; 2],
            strobe: false,
        }
    }
    /// Reset shift register for save state load.
    pub fn reset_runtime(&mut self) {
        self.index = [0; 2];
        self.strobe = false;
    }
}

impl Default for FamicomSet {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for FamicomSet {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 3 {
            self.cached = [state[0], state[1] & 0b11110011, state[2]];
        }
    }
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        let mic = address == 0 && self.cached[2] != 0;
        pad_common::read(
            &[self.cached[0], self.cached[1]],
            &mut self.index,
            self.strobe,
            address,
            mic,
        )
    }
    fn write(&mut self, value: u8) {
        pad_common::write(&mut self.strobe, &mut self.index, value);
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
