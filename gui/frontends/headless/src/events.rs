use serde::{Deserialize, Serialize};

use super::{
    error::RomTestError,
    serde_helpers::{hex_u8, hex_u16, hex_u64},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RomEvent {
    pub frame: u64,
    #[serde(flatten)]
    pub kind: RomEventKind,
}

impl RomEvent {
    pub(crate) fn validate(&self, case_id: &str) -> Result<(), RomTestError> {
        if let Some(assertion) = self.kind.assertion() {
            assertion.validate(case_id)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAssertionSpace {
    WorkRam,
    CartridgeRam,
    PpuVram,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RomAssertion {
    Screen {
        #[serde(with = "hex_u64")]
        hash: u64,
    },
    Memory {
        space: MemoryAssertionSpace,
        #[serde(with = "hex_u16")]
        address: u16,
        #[serde(with = "hex_u8")]
        value: u8,
        #[serde(default)]
        open_bus: bool,
    },
}

impl RomAssertion {
    fn validate(&self, case_id: &str) -> Result<(), RomTestError> {
        match self {
            RomAssertion::Screen { .. } => Ok(()),
            RomAssertion::Memory {
                space,
                address,
                open_bus,
                ..
            } => match space {
                MemoryAssertionSpace::WorkRam if *address > 0x1FFF => {
                    Err(RomTestError::InvalidManifest(format!(
                        "ROM case `{case_id}` uses check_work_ram outside CPU work RAM at address 0x{address:04X}"
                    )))
                }
                MemoryAssertionSpace::CartridgeRam if !(0x6000..=0x7FFF).contains(address) => {
                    Err(RomTestError::InvalidManifest(format!(
                        "ROM case `{case_id}` uses check_cartridge_ram outside cartridge RAM at address 0x{address:04X}"
                    )))
                }
                MemoryAssertionSpace::PpuVram if !(0x2000..=0x3FFF).contains(address) => {
                    Err(RomTestError::InvalidManifest(format!(
                        "ROM case `{case_id}` uses check_ppu_vram outside PPU nametable/palette space at address 0x{address:04X}"
                    )))
                }
                MemoryAssertionSpace::WorkRam | MemoryAssertionSpace::PpuVram if *open_bus => {
                    Err(RomTestError::InvalidManifest(format!(
                        "ROM case `{case_id}` uses open_bus with a non-cartridge memory assertion at address 0x{address:04X}"
                    )))
                }
                _ => Ok(()),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RomEventKind {
    Assert {
        #[serde(flatten)]
        assertion: RomAssertion,
    },
    CheckScreen {
        #[serde(with = "hex_u64")]
        hash: u64,
    },
    CheckWorkRam {
        #[serde(with = "hex_u16")]
        address: u16,
        #[serde(with = "hex_u8")]
        value: u8,
    },
    CheckCartridgeRam {
        #[serde(with = "hex_u16")]
        address: u16,
        #[serde(with = "hex_u8")]
        value: u8,
        #[serde(default)]
        open_bus: bool,
    },
    CheckPpuVram {
        #[serde(with = "hex_u16")]
        address: u16,
        #[serde(with = "hex_u8")]
        value: u8,
    },
    Reset,
    StandardController {
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    },
    Microphone {
        state: PadState,
    },
}

impl RomEventKind {
    pub(crate) fn assertion(&self) -> Option<RomAssertion> {
        match self {
            RomEventKind::Assert { assertion } => Some(assertion.clone()),
            RomEventKind::CheckScreen { hash } => Some(RomAssertion::Screen { hash: *hash }),
            RomEventKind::CheckWorkRam { address, value } => Some(RomAssertion::Memory {
                space: MemoryAssertionSpace::WorkRam,
                address: *address,
                value: *value,
                open_bus: false,
            }),
            RomEventKind::CheckCartridgeRam {
                address,
                value,
                open_bus,
            } => Some(RomAssertion::Memory {
                space: MemoryAssertionSpace::CartridgeRam,
                address: *address,
                value: *value,
                open_bus: *open_bus,
            }),
            RomEventKind::CheckPpuVram { address, value } => Some(RomAssertion::Memory {
                space: MemoryAssertionSpace::PpuVram,
                address: *address,
                value: *value,
                open_bus: false,
            }),
            RomEventKind::Reset
            | RomEventKind::StandardController { .. }
            | RomEventKind::Microphone { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerPad {
    Pad1,
    Pad2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonCode {
    A,
    B,
    SELECT,
    START,
    UP,
    DOWN,
    LEFT,
    RIGHT,
}

impl From<ButtonCode> for Buttons {
    fn from(value: ButtonCode) -> Self {
        match value {
            ButtonCode::A => Buttons::A,
            ButtonCode::B => Buttons::B,
            ButtonCode::SELECT => Buttons::SELECT,
            ButtonCode::START => Buttons::START,
            ButtonCode::UP => Buttons::UP,
            ButtonCode::DOWN => Buttons::DOWN,
            ButtonCode::LEFT => Buttons::LEFT,
            ButtonCode::RIGHT => Buttons::RIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadState {
    Pressed,
    Released,
}

bitflags::bitflags! {
    #[derive(
        serde::Serialize,
        serde::Deserialize,
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
    )]
    pub struct Buttons: u8 {
        const A =      0b0000_0001;
        const B =      0b0000_0010;
        const SELECT = 0b0000_0100;
        const START =  0b0000_1000;
        const UP =     0b0001_0000;
        const DOWN =   0b0010_0000;
        const LEFT =   0b0100_0000;
        const RIGHT =  0b1000_0000;
    }
}
