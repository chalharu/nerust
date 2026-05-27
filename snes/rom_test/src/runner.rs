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
    let should_wait_for_final_screen = case.expected_screen_hash.is_some();
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
                Ok(failures) if failures.is_empty() && !should_wait_for_final_screen => {
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
    mut failures: Vec<String>,
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
    match case.expected_screen_hash() {
        Ok(Some(expected_screen_hash)) if expected_screen_hash != final_screen_hash => {
            failures.push(format!(
                "screen_hash: expected 0x{expected_screen_hash:016X}, got 0x{final_screen_hash:016X}"
            ));
        }
        Ok(_) => {}
        Err(error) => {
            return internal_error(case, error.to_string());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::validate_case;
    use crate::manifest::load_manifest;
    use crate::results::CaseOutcome;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn unique_temp_path(name: &str, extension: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("nerust-snes-rom-test-{name}-{unique}.{extension}"))
    }

    fn write_test_rom(path: &PathBuf) {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST HASH ROM        ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2].copy_from_slice(&0x8000u16.to_le_bytes());
        rom[0x0000] = 0xEA;
        fs::write(path, rom).expect("test rom should be written");
    }

    #[test]
    fn expected_screen_hash_mismatch_is_reported_as_a_failure() {
        let rom_path = unique_temp_path("hash-mismatch", "sfc");
        let manifest_path = unique_temp_path("hash-mismatch", "yaml");
        write_test_rom(&rom_path);
        fs::write(
            &manifest_path,
            format!(
                "rom_root: .\ncases:\n  - id: hash-mismatch\n    description: Hash mismatch test\n    rom: {}\n    max_steps: 32\n    check_interval_steps: 16\n    expected_screen_hash: \"0x0000000000000000\"\n",
                rom_path.display()
            ),
        )
        .expect("manifest should be written");

        let manifest = load_manifest(&manifest_path).expect("manifest should load");
        let outcome = validate_case(manifest.case("hash-mismatch").expect("case should exist"));

        match outcome {
            CaseOutcome::Completed(validation) => {
                assert_eq!(validation.steps_executed, 32);
                assert!(
                    validation
                        .failures
                        .iter()
                        .any(|failure| failure.starts_with("screen_hash: expected")),
                    "expected screen hash failure, got {:?}",
                    validation.failures
                );
            }
            other => panic!("expected completed validation result, got {other:?}"),
        }

        fs::remove_file(rom_path).ok();
        fs::remove_file(manifest_path).ok();
    }
}
