// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::error::RomTestError;
use crate::events::{ButtonCode, ControllerPad, PadState};
use crate::harness::apply_button_state;
use crate::manifest::RomCase;
use crate::media::HashingMixer;
use crate::media::{encode_screenshot_png, screen_hash};
use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_core::Core;
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::logical_size::LogicalSize;
use nerust_sound_traits::MixerInput;

pub(super) struct ValidationRuntime {
    screen_buffer: ScreenBuffer,
    core: Core,
    mixer: HashingMixer,
    controller: StandardController,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
}

impl ValidationRuntime {
    pub(super) fn new(case: &RomCase, rom_bytes: &[u8]) -> Result<Self, RomTestError> {
        let cartridge_data =
            parse_cartridge_bytes(rom_bytes).map_err(|error| RomTestError::CoreConstruction {
                case_id: case.id.clone(),
                message: error.to_string(),
            })?;
        let core =
            Core::new_with_options(cartridge_data, case.core_options()).map_err(|error| {
                RomTestError::CoreConstruction {
                    case_id: case.id.clone(),
                    message: error.to_string(),
                }
            })?;

        Ok(Self {
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
        })
    }

    pub(super) fn run_frame(&mut self) -> u64 {
        let steps = self.core.run_frame(
            &mut self.screen_buffer,
            &mut self.controller,
            &mut self.mixer,
        );
        self.frame_counter += 1;
        steps
    }

    pub(super) fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    pub(super) fn reset(&mut self) {
        self.core.reset();
    }

    pub(super) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) {
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
    }

    pub(super) fn set_microphone(&mut self, state: PadState) {
        self.controller
            .set_microphone(matches!(state, PadState::Pressed));
    }

    pub(super) fn audio_sample_rate(&self) -> u32 {
        self.mixer.sample_rate()
    }

    pub(super) fn audio_samples(&self) -> u64 {
        self.mixer.samples()
    }

    pub(super) fn audio_hash(&self) -> u64 {
        self.mixer.checksum()
    }

    pub(super) fn screen_hash(&self) -> u64 {
        screen_hash(&self.screen_buffer)
    }

    pub(super) fn capture_screenshot_png(&self) -> Result<Vec<u8>, RomTestError> {
        encode_screenshot_png(&self.screen_buffer)
    }

    pub(super) fn peek_work_ram(&self, address: usize) -> Option<u8> {
        self.core.peek_work_ram(address)
    }

    pub(super) fn peek_cartridge_ram(&self, address: usize) -> Option<(u8, bool)> {
        self.core
            .peek_cartridge_ram(address)
            .map(|read_result| (read_result.data, read_result.mask != 0xFF))
    }

    pub(super) fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        self.core.peek_ppu_vram(address)
    }
}
