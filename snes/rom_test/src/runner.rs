// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::manifest::{Assertion, ManifestError, RomCase};
use crate::media::{SCREEN_HEIGHT, SCREEN_WIDTH, encode_screenshot_png, screen_hash_rgba};
use crate::render::render_screen;
use crate::results::{CaseOutcome, Validation, ValidationOptions};
use nerust_snes_core::{Core, CpuState};
use std::fs;

pub fn validate_case(case: &RomCase) -> CaseOutcome {
    validate_case_with_options(case, ValidationOptions::testing())
}

pub fn validate_case_with_options(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    let rom = match fs::read(case.rom_path()) {
        Ok(rom) => rom,
        Err(error) => {
            return internal_error(
                case,
                format!(
                    "failed to read ROM `{}`: {error}",
                    case.rom_path().display()
                ),
            );
        }
    };

    let mut core = match Core::from_rom_bytes(&rom) {
        Ok(core) => core,
        Err(error) => {
            return internal_error(
                case,
                format!(
                    "failed to construct SNES core from `{}`: {error}",
                    case.rom_path().display()
                ),
            );
        }
    };

    let mut steps_executed = 0_u64;
    while steps_executed < case.max_steps && core.current_state() != CpuState::Stopped {
        match core.step() {
            Ok(()) => {
                steps_executed += 1;
            }
            Err(error) => {
                return internal_error(
                    case,
                    format!("core error after {steps_executed} steps: {error}"),
                );
            }
        }

        if steps_executed.is_multiple_of(case.check_interval_steps) {
            match assertion_failures(case, &core) {
                Ok(failures) if failures.is_empty() => {
                    return finalize_validation(case, steps_executed, failures, &core, options);
                }
                Ok(_) => {}
                Err(error) => {
                    return internal_error(case, error.to_string());
                }
            }
        }
    }

    match assertion_failures(case, &core) {
        Ok(mut failures) => {
            if !failures.is_empty() {
                let reason = if core.current_state() == CpuState::Stopped {
                    format!("core stopped after {steps_executed} steps before expectations matched")
                } else {
                    format!("expectations did not match within {} steps", case.max_steps)
                };
                failures.insert(0, reason);
            }
            finalize_validation(case, steps_executed, failures, &core, options)
        }
        Err(error) => internal_error(case, error.to_string()),
    }
}

fn finalize_validation(
    case: &RomCase,
    steps_executed: u64,
    failures: Vec<String>,
    core: &Core,
    options: ValidationOptions,
) -> CaseOutcome {
    let rendered = match render_screen(core) {
        Ok(rendered) => rendered,
        Err(error) => {
            return internal_error(
                case,
                format!("failed to render final screen after {steps_executed} steps: {error}"),
            );
        }
    };
    let final_screen_hash = screen_hash_rgba(&rendered.rgba);
    let screenshot_png = if options.capture_screenshot_png {
        match encode_screenshot_png(&rendered.rgba, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                return internal_error(
                    case,
                    format!(
                        "failed to encode final screenshot after {steps_executed} steps: {error}"
                    ),
                );
            }
        }
    } else {
        None
    };

    CaseOutcome::Completed(Validation {
        case_id: case.id.clone(),
        description: case.description.clone(),
        rom: case.rom_path().display().to_string(),
        steps_executed,
        final_screen_hash,
        screenshot_png,
        failures,
    })
}

fn internal_error(case: &RomCase, message: String) -> CaseOutcome {
    CaseOutcome::InternalError {
        case_id: case.id.clone(),
        description: case.description.clone(),
        rom: case.rom_path().display().to_string(),
        message,
    }
}

fn assertion_failures(case: &RomCase, core: &Core) -> Result<Vec<String>, ManifestError> {
    case.assertions
        .iter()
        .map(|assertion| match assertion {
            Assertion::BusU8 { .. }
            | Assertion::WramU8 { .. }
            | Assertion::VramU8 { .. }
            | Assertion::CgramU8 { .. }
            | Assertion::OamU8 { .. } => evaluate_u8_assertion(assertion, core),
            Assertion::BusU16 { .. }
            | Assertion::WramU16 { .. }
            | Assertion::VramU16 { .. }
            | Assertion::CgramU16 { .. }
            | Assertion::OamU16 { .. } => evaluate_u16_assertion(assertion, core),
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
        Assertion::OamU8 { .. } => core.peek_oam(address as usize),
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
        Assertion::OamU16 { .. } => u16::from_le_bytes([
            core.peek_oam(address as usize),
            core.peek_oam(address as usize + 1),
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
        Assertion::OamU8 { .. } => "oam_u8",
        Assertion::OamU16 { .. } => "oam_u16",
    }
}
