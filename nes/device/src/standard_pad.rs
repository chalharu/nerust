use nerust_input_traits::{AbstractKey, ControlInfo, ControlKind, ControllerProfile, PortSet};
use nerust_nes_core::{OpenBusReadResult, controller::Controller};

use crate::pad_common;

/// NES Standard Controller: full 8-button pad for a single port.
#[derive(Debug, Clone)]
pub struct StandardPad {
    pub(crate) cached: u8,
    pub(crate) result: u8,
    pub(crate) strobe: bool,
    /// Open bus mask for this port ($4016=3, $4017=1).
    open_bus_mask: u8,
}

impl StandardPad {
    pub fn new(open_bus_mask: u8) -> Self {
        Self {
            cached: 0,
            result: 0,
            strobe: false,
            open_bus_mask,
        }
    }
    /// Reset shift register for save state load.
    pub fn reset_runtime(&mut self) {
        self.result = 0;
        self.strobe = false;
    }
}

impl Default for StandardPad {
    fn default() -> Self {
        Self::new(3)
    }
}

impl Controller for StandardPad {
    fn sync_input(&mut self, state: &[u8]) {
        if let Some(&b) = state.first() {
            self.cached = b;
        }
    }
    fn read(&mut self) -> OpenBusReadResult {
        let bit = if self.strobe {
            self.cached & 1
        } else {
            let b = self.result & 1;
            self.result = self.result >> 1 | 0x80;
            b
        };
        OpenBusReadResult::new(bit, self.open_bus_mask)
    }
    fn write(&mut self, value: u8) {
        pad_common::write(&mut self.strobe, &self.cached, &mut self.result, value);
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
