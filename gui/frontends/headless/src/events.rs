use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    error::RomTestError,
    serde_helpers::{hex_u8, hex_u32, hex_u64, hex_u64_map},
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RomAssertion {
    Screen {
        #[serde(with = "hex_u64")]
        hash: u64,
    },
    /// Plain memory assertion. The containing space resolves by
    /// address against the debugger table snapshot at read time, so
    /// new spaces need no schema change. 32-bit: GBA-class buses
    /// need the full range; 16-bit systems simply never use the top.
    Memory {
        #[serde(with = "hex_u32")]
        address: u32,
        #[serde(with = "hex_u8")]
        value: u8,
        /// Expect a floating bus instead of a mapped value. Only
        /// meaningful where hardware can float (cartridge RAM); the
        /// read resolves open-vs-mapped through the Spaces table.
        #[serde(default)]
        open_bus: bool,
    },
    /// Register assertion. Names resolve against the debugger's live
    /// register list at read time, so new registers need no schema
    /// change. Only listed registers compare; extras are ignored.
    Registers {
        #[serde(with = "hex_u64_map")]
        registers: BTreeMap<String, u64>,
    },
}
impl RomAssertion {
    fn validate(&self, _case_id: &str) -> Result<(), RomTestError> {
        match self {
            // Address coverage validates at read time against the live
            // table snapshot; unmapped addresses fail loudly there
            // with the same error kind.
            RomAssertion::Screen { .. } => Ok(()),
            RomAssertion::Memory { .. } => Ok(()),
            // Register name coverage validates at read time against
            // the live register list; unknown names fail loudly there.
            RomAssertion::Registers { .. } => Ok(()),
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
    CheckMemory {
        #[serde(with = "hex_u32")]
        address: u32,
        #[serde(with = "hex_u8")]
        value: u8,
        #[serde(default)]
        open_bus: bool,
    },
    CheckRegisters {
        #[serde(with = "hex_u64_map")]
        registers: BTreeMap<String, u64>,
    },
    Reset,
    StandardController {
        pad: ControllerPad,
        /// Control id string as exposed by the slot profile group
        /// (e.g. `"nes.control.a"`). Validated against the live groups
        /// at open; unknown ids are loud errors, never guesses.
        button: String,
        state: PadState,
    },
}

impl RomEventKind {
    pub(crate) fn assertion(&self) -> Option<RomAssertion> {
        match self {
            RomEventKind::Assert { assertion } => Some(assertion.clone()),
            RomEventKind::CheckScreen { hash } => Some(RomAssertion::Screen { hash: *hash }),
            RomEventKind::CheckMemory {
                address,
                value,
                open_bus,
            } => Some(RomAssertion::Memory {
                address: *address,
                value: *value,
                open_bus: *open_bus,
            }),
            RomEventKind::CheckRegisters { registers } => Some(RomAssertion::Registers {
                registers: registers.clone(),
            }),
            RomEventKind::Reset | RomEventKind::StandardController { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerPad {
    Pad1,
    Pad2,
}

impl ControllerPad {
    /// Slot index into the input assignments.
    pub fn index(self) -> usize {
        match self {
            ControllerPad::Pad1 => 0,
            ControllerPad::Pad2 => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadState {
    Pressed,
    Released,
}
