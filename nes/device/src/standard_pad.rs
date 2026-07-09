use nerust_input_traits::{AbstractKey, ControlInfo, ControlKind, ControllerProfile, PortSet};
use nerust_nes_core::{OpenBusReadResult, controller::Controller};

use crate::pad_common;

/// NES Standard Controller: full 8-button pad on both ports.
#[derive(Debug, Clone)]
pub struct StandardPad {
    pub(crate) cached: [u8; 2],
    pub(crate) result: [u8; 2],
    pub(crate) strobe: bool,
}

impl StandardPad {
    pub fn new() -> Self {
        Self {
            cached: [0; 2],
            result: [0; 2],
            strobe: false,
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
        Self::new()
    }
}

impl Controller for StandardPad {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 2 {
            self.cached = [state[0], state[1]];
        }
    }
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        match address {
            // $4016
            0 => {
                // strobe が 1 のときは常に最初のボタンの状態を返す
                // strobe が 0 のときはシフトレジスタの状態を返す
                let bit = if self.strobe {
                    self.cached[0] & 1
                } else {
                    let b = self.result[0] & 1;
                    self.result[0] = self.result[0] >> 1 | 0x80; // シフトレジスタを右にシフトして、上位ビットを 1 にする
                    b
                };
                // StandardPadではmicはないので、Port解放状態となる
                OpenBusReadResult::new(bit, 3)
            }
            // $4017
            1 => {
                let bit = if self.strobe {
                    self.cached[1] & 1
                } else {
                    let b = self.result[1] & 1;
                    self.result[1] = self.result[1] >> 1 | 0x80; // シフトレジスタを右にシフトして、上位ビットを 1 にする
                    b
                };
                OpenBusReadResult::new(bit, 1)
            }
            _ => unreachable!("invalid controller read address: 0x{:04X}", address),
        }
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
