// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::manifest::{Assertion, ManifestError, RomCase};
use crate::results::{CaseOutcome, Validation};
use nerust_snes_core::{Core, CpuState};
use std::fs;

pub fn validate_case(case: &RomCase) -> CaseOutcome {
    let rom = match fs::read(case.rom_path()) {
        Ok(rom) => rom,
        Err(error) => {
            return CaseOutcome::InternalError {
                case_id: case.id.clone(),
                message: format!(
                    "failed to read ROM `{}`: {error}",
                    case.rom_path().display()
                ),
            };
        }
    };

    let mut core = match Core::from_rom_bytes(&rom) {
        Ok(core) => core,
        Err(error) => {
            return CaseOutcome::InternalError {
                case_id: case.id.clone(),
                message: format!(
                    "failed to construct SNES core from `{}`: {error}",
                    case.rom_path().display()
                ),
            };
        }
    };

    let mut steps_executed = 0_u64;
    while steps_executed < case.max_steps && core.current_state() != CpuState::Stopped {
        match core.step() {
            Ok(()) => {
                steps_executed += 1;
            }
            Err(error) => {
                return CaseOutcome::InternalError {
                    case_id: case.id.clone(),
                    message: format!("core error after {steps_executed} steps: {error}"),
                };
            }
        }

        if steps_executed.is_multiple_of(case.check_interval_steps) {
            match assertion_failures(case, &core) {
                Ok(failures) if failures.is_empty() => {
                    return CaseOutcome::Completed(Validation {
                        case_id: case.id.clone(),
                        steps_executed,
                        failures,
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    return CaseOutcome::InternalError {
                        case_id: case.id.clone(),
                        message: error.to_string(),
                    };
                }
            }
        }
    }

    match assertion_failures(case, &core) {
        Ok(mut failures) => {
            if failures.is_empty() {
                CaseOutcome::Completed(Validation {
                    case_id: case.id.clone(),
                    steps_executed,
                    failures,
                })
            } else {
                let reason = if core.current_state() == CpuState::Stopped {
                    format!("core stopped after {steps_executed} steps before expectations matched")
                } else {
                    format!("expectations did not match within {} steps", case.max_steps)
                };
                failures.insert(0, reason);
                CaseOutcome::Completed(Validation {
                    case_id: case.id.clone(),
                    steps_executed,
                    failures,
                })
            }
        }
        Err(error) => CaseOutcome::InternalError {
            case_id: case.id.clone(),
            message: error.to_string(),
        },
    }
}

fn assertion_failures(case: &RomCase, core: &Core) -> Result<Vec<String>, ManifestError> {
    case.assertions
        .iter()
        .map(|assertion| match assertion {
            Assertion::BusU8 { .. }
            | Assertion::WramU8 { .. }
            | Assertion::VramU8 { .. }
            | Assertion::CgramU8 { .. } => evaluate_u8_assertion(assertion, core),
            Assertion::BusU16 { .. }
            | Assertion::WramU16 { .. }
            | Assertion::VramU16 { .. }
            | Assertion::CgramU16 { .. } => evaluate_u16_assertion(assertion, core),
        })
        .filter_map(Result::transpose)
        .collect()
}

fn evaluate_u8_assertion(
    assertion: &Assertion,
    core: &Core,
) -> Result<Option<String>, ManifestError> {
    let address = assertion.address()?;
    let expected = assertion.expected_u8()?;
    let actual = match assertion {
        Assertion::BusU8 { .. } => core.peek(address),
        Assertion::WramU8 { .. } => core.peek_wram(address as usize),
        Assertion::VramU8 { .. } => core.peek_vram(address as usize),
        Assertion::CgramU8 { .. } => core.peek_cgram(address as usize),
        _ => {
            return Err(ManifestError::Invalid {
                message: "evaluate_u8_assertion called for 16-bit assertion".to_string(),
            });
        }
    };

    if actual == expected {
        Ok(None)
    } else {
        Ok(Some(format!(
            "{} @ 0x{address:06X}: expected 0x{expected:02X}, got 0x{actual:02X}",
            assertion_kind(assertion)
        )))
    }
}

fn evaluate_u16_assertion(
    assertion: &Assertion,
    core: &Core,
) -> Result<Option<String>, ManifestError> {
    let address = assertion.address()?;
    let expected = assertion.expected_u16()?;
    let actual = match assertion {
        Assertion::BusU16 { .. } => {
            u16::from_le_bytes([core.peek(address), core.peek(address + 1)])
        }
        Assertion::WramU16 { .. } => u16::from_le_bytes([
            core.peek_wram(address as usize),
            core.peek_wram(address as usize + 1),
        ]),
        Assertion::VramU16 { .. } => u16::from_le_bytes([
            core.peek_vram(address as usize),
            core.peek_vram(address as usize + 1),
        ]),
        Assertion::CgramU16 { .. } => u16::from_le_bytes([
            core.peek_cgram(address as usize),
            core.peek_cgram(address as usize + 1),
        ]),
        _ => {
            return Err(ManifestError::Invalid {
                message: "evaluate_u16_assertion called for 8-bit assertion".to_string(),
            });
        }
    };

    if actual == expected {
        Ok(None)
    } else {
        Ok(Some(format!(
            "{} @ 0x{address:06X}: expected 0x{expected:04X}, got 0x{actual:04X}",
            assertion_kind(assertion)
        )))
    }
}

fn assertion_kind(assertion: &Assertion) -> &'static str {
    match assertion {
        Assertion::BusU8 { .. } => "bus_u8",
        Assertion::BusU16 { .. } => "bus_u16",
        Assertion::WramU8 { .. } => "wram_u8",
        Assertion::WramU16 { .. } => "wram_u16",
        Assertion::VramU8 { .. } => "vram_u8",
        Assertion::VramU16 { .. } => "vram_u16",
        Assertion::CgramU8 { .. } => "cgram_u8",
        Assertion::CgramU16 { .. } => "cgram_u16",
    }
}
