// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::error::RomTestError;
use super::events::{
    ButtonCode, ControllerPad, MemoryAssertionSpace, PadState, RomAssertion, RomEventKind,
};
use super::manifest::{RomCase, read_rom};
use super::media::{HashingMixer, encode_screenshot_png, screen_hash};
use super::results::{
    AudioObservation, CartridgeRamCheck, CaseOutcome, CaseValidation, ExecutionTotals,
    PpuVramCheck, ScreenCheck, ValidationOptions, WorkRamCheck,
};
use nerust_core::Core;
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::LogicalSize;
use nerust_sound_traits::MixerInput;

pub trait CaseHarness {
    fn run_frame(&mut self) -> u64;
    fn frame_counter(&self) -> u64;
    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError>;
    fn on_reset(&mut self) -> Result<(), RomTestError>;
    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), RomTestError>;
    fn on_microphone(&mut self, state: PadState) -> Result<(), RomTestError>;
}

pub fn drive_case<H: CaseHarness>(
    case: &RomCase,
    harness: &mut H,
) -> Result<ExecutionTotals, RomTestError> {
    let final_frame = case.final_frame();
    let mut total_steps = 0_u64;
    let mut next_event = 0_usize;

    dispatch_pending_events(case, harness, &mut next_event)?;

    while harness.frame_counter() < final_frame {
        total_steps += harness.run_frame();
        dispatch_pending_events(case, harness, &mut next_event)?;
    }

    Ok(ExecutionTotals {
        frames: harness.frame_counter(),
        steps: total_steps,
    })
}

struct ValidationRunner {
    case_id: String,
    screen_buffer: ScreenBuffer,
    core: Core,
    controller: StandardController,
    mixer: HashingMixer,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    screen_checks: Vec<ScreenCheck>,
    work_ram_checks: Vec<WorkRamCheck>,
    cartridge_ram_checks: Vec<CartridgeRamCheck>,
    ppu_vram_checks: Vec<PpuVramCheck>,
    failures: Vec<String>,
    options: ValidationOptions,
}

impl ValidationRunner {
    fn new(
        case: &RomCase,
        rom_bytes: &[u8],
        options: ValidationOptions,
    ) -> Result<Self, RomTestError> {
        let mut input = rom_bytes.iter().copied();
        let core = Core::new_with_options(&mut input, case.core_options()).map_err(|error| {
            RomTestError::CoreConstruction {
                case_id: case.id.clone(),
                message: error.to_string(),
            }
        })?;

        Ok(Self {
            case_id: case.id.clone(),
            screen_buffer: ScreenBuffer::new(
                FilterType::None,
                LogicalSize {
                    width: 256,
                    height: 240,
                },
            ),
            core,
            controller: StandardController::new(),
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            screen_checks: Vec::new(),
            work_ram_checks: Vec::new(),
            cartridge_ram_checks: Vec::new(),
            ppu_vram_checks: Vec::new(),
            failures: Vec::new(),
            options,
        })
    }

    fn run_case(mut self, case: &RomCase) -> Result<CaseValidation, RomTestError> {
        let totals = drive_case(case, &mut self)?;
        let final_screen_hash = screen_hash(&self.screen_buffer);
        let audio = AudioObservation {
            sample_rate: self.mixer.sample_rate(),
            samples: self.mixer.samples(),
            hash: self.mixer.checksum(),
            expected: case.expected_audio.clone(),
        };

        if self.options.check_expectations
            && let Some(expected_audio) = &audio.expected
        {
            if audio.samples != expected_audio.samples {
                self.failures.push(format!(
                    "{}: audio sample mismatch (expected {}, actual {})",
                    case.id, expected_audio.samples, audio.samples
                ));
            }
            if audio.hash != expected_audio.hash {
                self.failures.push(format!(
                    "{}: audio hash mismatch (expected 0x{:016X}, actual 0x{:016X})",
                    case.id, expected_audio.hash, audio.hash
                ));
            }
        }

        Ok(CaseValidation {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            frames: totals.frames,
            steps: totals.steps,
            final_screen_hash,
            screen_checks: self.screen_checks,
            work_ram_checks: self.work_ram_checks,
            cartridge_ram_checks: self.cartridge_ram_checks,
            ppu_vram_checks: self.ppu_vram_checks,
            audio,
            failures: self.failures,
        })
    }

    fn record_screen_assert(&mut self, frame: u64, expected_hash: u64) -> Result<(), RomTestError> {
        let actual_hash = screen_hash(&self.screen_buffer);
        if self.options.check_expectations && actual_hash != expected_hash {
            self.failures.push(format!(
                "{}: screen hash mismatch at frame {} (expected 0x{:016X}, actual 0x{:016X})",
                self.case_id, frame, expected_hash, actual_hash
            ));
        }

        let screenshot_png = if self.options.capture_screenshots {
            Some(encode_screenshot_png(&self.screen_buffer)?)
        } else {
            None
        };

        self.screen_checks.push(ScreenCheck {
            frame,
            expected_hash,
            actual_hash,
            screenshot_png,
        });
        Ok(())
    }

    fn record_work_ram_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
    ) -> Result<(), RomTestError> {
        let actual_value = self.core.peek_work_ram(address).ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "ROM case `{}` requested check_work_ram outside CPU work RAM at address 0x{address:04X}",
                self.case_id
            ))
        })?;
        if self.options.check_expectations && actual_value != expected_value {
            self.failures.push(format!(
                "{}: work RAM mismatch at frame {} address 0x{:04X} (expected 0x{:02X}, actual 0x{:02X})",
                self.case_id, frame, address, expected_value, actual_value
            ));
        }

        self.work_ram_checks.push(WorkRamCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
        });
        Ok(())
    }

    fn record_cartridge_ram_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
        expect_open_bus: bool,
    ) -> Result<(), RomTestError> {
        let read_result = self.core.peek_cartridge_ram(address).ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "ROM case `{}` requested check_cartridge_ram outside cartridge RAM at address 0x{address:04X}",
                self.case_id
            ))
        })?;
        let actual_value = read_result.data;
        let actual_open_bus = read_result.mask != 0xFF;
        if self.options.check_expectations && actual_open_bus != expect_open_bus {
            self.failures.push(format!(
                "{}: cartridge RAM bus state mismatch at frame {} address 0x{:04X} (expected {}, actual {})",
                self.case_id,
                frame,
                address,
                if expect_open_bus { "open bus" } else { "mapped RAM" },
                if actual_open_bus { "open bus" } else { "mapped RAM" }
            ));
        }
        if self.options.check_expectations && !expect_open_bus && actual_value != expected_value {
            self.failures.push(format!(
                "{}: cartridge RAM mismatch at frame {} address 0x{:04X} (expected 0x{:02X}, actual 0x{:02X})",
                self.case_id, frame, address, expected_value, actual_value
            ));
        }

        self.cartridge_ram_checks.push(CartridgeRamCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
            expected_open_bus: expect_open_bus,
            actual_open_bus,
        });
        Ok(())
    }

    fn record_ppu_vram_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
    ) -> Result<(), RomTestError> {
        let actual_value = self.core.peek_ppu_vram(address).ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "ROM case `{}` requested check_ppu_vram outside PPU nametable/palette space at address 0x{address:04X}",
                self.case_id
            ))
        })?;
        if self.options.check_expectations && actual_value != expected_value {
            self.failures.push(format!(
                "{}: PPU VRAM mismatch at frame {} address 0x{:04X} (expected 0x{:02X}, actual 0x{:02X})",
                self.case_id, frame, address, expected_value, actual_value
            ));
        }

        self.ppu_vram_checks.push(PpuVramCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
        });
        Ok(())
    }
}

impl CaseHarness for ValidationRunner {
    fn run_frame(&mut self) -> u64 {
        let steps = self.core.run_frame(
            &mut self.screen_buffer,
            &mut self.controller,
            &mut self.mixer,
        );
        self.frame_counter += 1;
        steps
    }

    fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError> {
        match assertion {
            RomAssertion::Screen { hash } => self.record_screen_assert(frame, *hash),
            RomAssertion::Memory {
                space,
                address,
                value,
                open_bus,
            } => match space {
                MemoryAssertionSpace::WorkRam => {
                    self.record_work_ram_assert(frame, usize::from(*address), *value)
                }
                MemoryAssertionSpace::CartridgeRam => self.record_cartridge_ram_assert(
                    frame,
                    usize::from(*address),
                    *value,
                    *open_bus,
                ),
                MemoryAssertionSpace::PpuVram => {
                    self.record_ppu_vram_assert(frame, usize::from(*address), *value)
                }
            },
        }
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.core.reset();
        Ok(())
    }

    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), RomTestError> {
        let buttons = Buttons::from(button);
        match pad {
            ControllerPad::Pad1 => {
                self.pad1 = apply_button_state(self.pad1, buttons, state);
                self.controller.set_pad1(self.pad1);
            }
            ControllerPad::Pad2 => {
                self.pad2 = apply_button_state(self.pad2, buttons, state);
                self.controller.set_pad2(self.pad2);
            }
        }
        Ok(())
    }

    fn on_microphone(&mut self, state: PadState) -> Result<(), RomTestError> {
        self.controller
            .set_microphone(matches!(state, PadState::Pressed));
        Ok(())
    }
}

pub fn validate_case(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    match read_rom(case)
        .and_then(|rom_bytes| ValidationRunner::new(case, &rom_bytes, options)?.run_case(case))
    {
        Ok(validation) => CaseOutcome::Completed(validation),
        Err(error) => CaseOutcome::InternalError {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            message: error.to_string(),
        },
    }
}

fn apply_button_state(current: Buttons, button: Buttons, state: PadState) -> Buttons {
    match state {
        PadState::Pressed => current | button,
        PadState::Released => current & !button,
    }
}

fn dispatch_pending_events<H: CaseHarness>(
    case: &RomCase,
    harness: &mut H,
    next_event: &mut usize,
) -> Result<(), RomTestError> {
    while let Some(event) = case.events.get(*next_event) {
        if event.frame != harness.frame_counter() {
            break;
        }

        if let Some(assertion) = event.kind.assertion() {
            harness.on_assert(event.frame, &assertion)?;
        } else {
            match event.kind {
                RomEventKind::Reset => {
                    harness.on_reset()?;
                }
                RomEventKind::StandardController { pad, button, state } => {
                    harness.on_standard_controller(pad, button, state)?;
                }
                RomEventKind::Microphone { state } => {
                    harness.on_microphone(state)?;
                }
                RomEventKind::Assert { .. }
                | RomEventKind::CheckScreen { .. }
                | RomEventKind::CheckWorkRam { .. }
                | RomEventKind::CheckCartridgeRam { .. }
                | RomEventKind::CheckPpuVram { .. } => unreachable!(),
            }
        }

        *next_event += 1;
    }

    Ok(())
}
