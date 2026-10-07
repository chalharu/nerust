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

/// Default output channel: the link-cable serial stream. Keeps the
/// 1057 existing serial pins working untouched; log-sink cases name
/// their channel explicitly.
fn default_serial_channel() -> String {
    "serial".to_string()
}

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
///
/// 2026-10 update: a second output channel (a guest debug-log sink)
/// did NOT become kind 5. A channel key on `Serial` covers named byte
/// streams without a new variant: still snapshot-shaped, still the
/// same dispatch. The trigger remains reserved for a genuinely new
/// (non-snapshot) kind.
///
/// 2026-10 update 2: line-set log evaluation DID become kind 5, as a
/// `Log` variant reusing the same dispatch shape (`assertion()`
/// mapping, same `on_assert` path). Cumulative-transcript set
/// evaluation is snapshot family, so still no dispatcher registry;
/// the trigger stays reserved for a genuinely non-snapshot kind.
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
    /// core produced on one named output channel since power-on,
    /// exactly. Streams are stateful but cumulative-at-a-frame is
    /// still a snapshot, so no new dispatch shape was needed. The
    /// channel is core-defined (e.g. `"serial"`, `"debug-log"`) and
    /// validated at read time against the live channel list; cores
    /// producing nothing on it stay silent (empty), and asserting
    /// non-empty bytes there fails like any other mismatch.
    Serial {
        #[serde(default = "default_serial_channel")]
        channel: String,
        #[serde(with = "hex_bytes")]
        bytes: Vec<u8>,
    },
    /// Log-line assertion. Evaluates the cumulative
    /// newline-delimited text on `channel` at a frame: the exact
    /// `end` marker line must be present (completion proof; it
    /// carries the pass/total counts), and the set of `fail_prefix`-led
    /// test names must equal `allowed_fail` exactly. Unknown failures
    /// fail; allowed names that no longer fail are stale (always
    /// hard — a fix must never stay silent under a tolerance flag).
    ///
    /// The line format is DATA, not code: `fail_prefix`/`end` come
    /// from the manifest (cf. legacy `SuiteLogVerify`), so no suite's
    /// printf dialect lives in generic code. Detail lines
    /// (actual-vs-expected values) are evidence only, never matched:
    /// pinning wrong values would be pinning bad output. PASS lines
    /// are not enumerated; the exact `end` line guards count drift.
    Log {
        #[serde(default = "default_serial_channel")]
        channel: String,
        end: String,
        fail_prefix: String,
        #[serde(default)]
        allowed_fail: Vec<String>,
    },
}
impl RomAssertion {
    fn validate(&self, case_id: &str) -> Result<(), RomTestError> {
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
            // Line-set shape validates structurally: end marker and
            // fail prefix must be non-empty (an empty prefix would
            // match every line; an empty end marker proves nothing).
            RomAssertion::Log {
                end, fail_prefix, ..
            } => {
                if end.is_empty() || fail_prefix.is_empty() {
                    return Err(RomTestError::InvalidManifest(format!(
                        "{case_id}: check_log needs non-empty end and fail_prefix"
                    )));
                }
                Ok(())
            }
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
        #[serde(default = "default_serial_channel")]
        channel: String,
        #[serde(with = "hex_bytes")]
        bytes: Vec<u8>,
    },
    CheckLog {
        #[serde(default = "default_serial_channel")]
        channel: String,
        end: String,
        fail_prefix: String,
        #[serde(default)]
        allowed_fail: Vec<String>,
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
            RomEventKind::CheckSerial { channel, bytes } => Some(RomAssertion::Serial {
                channel: channel.clone(),
                bytes: bytes.clone(),
            }),
            RomEventKind::CheckLog {
                channel,
                end,
                fail_prefix,
                allowed_fail,
            } => Some(RomAssertion::Log {
                channel: channel.clone(),
                end: end.clone(),
                fail_prefix: fail_prefix.clone(),
                allowed_fail: allowed_fail.clone(),
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
