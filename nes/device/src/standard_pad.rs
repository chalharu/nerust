use nerust_input_traits::{AbstractKey, ControlInfo, ControlKind, ControllerProfile, PortSet};
use nerust_nes_core::{OpenBusReadResult, controller::Controller};

use crate::pad_common;

/// NES Standard Controller: full 8-button pad on both ports.
#[derive(Debug, Clone)]
pub struct StandardPad {
    pub(crate) cached: [u8; 2],
    pub(crate) index: [u8; 2],
    pub(crate) strobe: bool,
}

impl StandardPad {
    pub fn new() -> Self {
        Self { cached: [0; 2], index: [0; 2], strobe: false }
    }
    /// Reset shift register for save state load.
    pub fn reset_runtime(&mut self) {
        self.index = [0; 2];
        self.strobe = false;
    }
}

impl Default for StandardPad {
    fn default() -> Self { Self::new() }
}

impl Controller for StandardPad {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 2 { self.cached = [state[0], state[1]]; }
    }
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        pad_common::read(&self.cached, &mut self.index, self.strobe, address, false)
    }
    fn write(&mut self, value: u8) {
        pad_common::write(&mut self.strobe, &mut self.index, value);
    }
}

#[derive(Debug)]
pub struct StandardPadProfile;

impl ControllerProfile for StandardPadProfile {
    fn id(&self) -> &'static str { "nes.standard_pad" }
    fn label(&self) -> &'static str { "NES Standard Controller" }
    fn port_sets(&self) -> &[PortSet] {
        &[PortSet { ports: &["player1"] }, PortSet { ports: &["player2"] }]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::*;
        static C: &[ControlInfo] = &[
            ControlInfo { id: "a", label: "A", kind: Digital, abstract_key: Some(AbstractKey::Button1) },
            ControlInfo { id: "b", label: "B", kind: Digital, abstract_key: Some(AbstractKey::Button2) },
            ControlInfo { id: "select", label: "Select", kind: Digital, abstract_key: Some(AbstractKey::Select) },
            ControlInfo { id: "start", label: "Start", kind: Digital, abstract_key: Some(AbstractKey::Start) },
            ControlInfo { id: "up", label: "Up", kind: Digital, abstract_key: Some(AbstractKey::DpadUp) },
            ControlInfo { id: "down", label: "Down", kind: Digital, abstract_key: Some(AbstractKey::DpadDown) },
            ControlInfo { id: "left", label: "Left", kind: Digital, abstract_key: Some(AbstractKey::DpadLeft) },
            ControlInfo { id: "right", label: "Right", kind: Digital, abstract_key: Some(AbstractKey::DpadRight) },
        ];
        static G: &[&[ControlInfo]] = &[C];
        G
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}
