use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    error::RomTestError,
    serde_helpers::{hex_bytes, hex_u8, hex_u32, hex_u64, hex_u64_map},
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

/// Test-domain assertion vocabulary. New kinds need no schema
/// migration for addresses/names (those resolve against live system
/// state), but adding a variant is intentionally shotgun: every `match`
/// below must handle it, so the compiler lists all touch points.
///
/// Extension checklist for kind N+1 (serial touched the same
/// 14 files as registers):
/// `serde_helpers` codec → this enum + `RomEventKind` + `assertion()`
/// → `harness.rs` dispatch → `TestSystem` read → `runtime/inspection`
/// peek → `runner.rs` record → `harness_impl` arm → `artifacts/`
/// module + wiring → `results.rs` check + `CaseValidation` field →
/// `summary.rs` → `report.rs` → `bin/rom_tool.rs` → `tests.rs`
/// (+ kernel/thread only if a new `EmuCommand` is needed — serial did:
/// `TapSerialOutput`, mirroring the audio tap).
///
/// Do NOT abstract this into a dispatcher registry until kind 5.
/// Exhaustive matches fail at compile time when a variant is missed;
/// a registry would fail at runtime instead. Serial (kind 4) still
/// fits snapshot-compare — cumulative bytes at a frame are a snapshot
/// — so the revisit trigger moves to the next stateful kind.
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
    /// Serial-output assertion. Compares the cumulative bytes the
    /// core transmitted since power-on, exactly. Streams are stateful
    /// but cumulative-at-a-frame is still a snapshot, so no new
    /// dispatch shape was needed. Cores without a serial port stay
    /// silent (empty); asserting non-empty bytes there fails like any
    /// other mismatch.
    Serial {
        #[serde(with = "hex_bytes")]
        bytes: Vec<u8>,
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
            // Byte content compares at read time against the cumulative
            // tap; no names or addresses to pre-validate.
            RomAssertion::Serial { .. } => Ok(()),
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
    CheckSerial {
        #[serde(with = "hex_bytes")]
        bytes: Vec<u8>,
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
            RomEventKind::CheckSerial { bytes } => Some(RomAssertion::Serial {
                bytes: bytes.clone(),
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
