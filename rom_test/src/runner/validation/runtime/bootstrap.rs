// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::ValidationRuntime;
use crate::error::RomTestError;
use crate::manifest::RomCase;
use crate::media::HashingMixer;
use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_core::Core;
use nerust_input_nes::frame::Buttons;
use nerust_input_nes_runtime::StandardController;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_logical::LogicalSize;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
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
            screen_buffer: validation_screen_buffer(),
            core,
            controller: StandardController::new(),
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
        })
    }
}

fn validation_screen_buffer() -> ScreenBuffer {
    ScreenBuffer::new(
        FilterType::None,
        LogicalSize {
            width: 256,
            height: 240,
        },
    )
}
