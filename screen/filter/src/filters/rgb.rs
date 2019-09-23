// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::FilterUnit;
use nerust_screen_traits::{LogicalSize, PhysicalSize, RGB};

const PALETTE: [u32; 64] = [
    0x66_6666, 0x00_2A88, 0x14_12A7, 0x3B_00A4, 0x5C_007E, 0x6E_0040, 0x6C_0600, 0x56_1D00,
    0x33_3500, 0x0B_4800, 0x00_5200, 0x00_4F08, 0x00_404D, 0x00_0000, 0x00_0000, 0x00_0000,
    0xAD_ADAD, 0x15_5FD9, 0x42_40FF, 0x75_27FE, 0xA0_1ACC, 0xB7_1E7B, 0xB5_3120, 0x99_4E00,
    0x6B_6D00, 0x38_8700, 0x0C_9300, 0x00_8F32, 0x00_7C8D, 0x00_0000, 0x00_0000, 0x00_0000,
    0xFF_FEFF, 0x64_B0FF, 0x92_90FF, 0xC6_76FF, 0xF3_6AFF, 0xFE_6ECC, 0xFE_8170, 0xEA_9E22,
    0xBC_BE00, 0x88_D800, 0x5C_E430, 0x45_E082, 0x48_CDDE, 0x4F_4F4F, 0x00_0000, 0x00_0000,
    0xFF_FEFF, 0xC0_DFFF, 0xD3_D2FF, 0xE8_C8FF, 0xFB_C2FF, 0xFE_C4EA, 0xFE_CCC5, 0xF7_D8A5,
    0xE4_E594, 0xCF_EF96, 0xBD_F4AB, 0xB3_F3CC, 0xB5_EBF2, 0xB8_B8B8, 0x00_0000, 0x00_0000,
];

#[derive(Debug)]
pub(crate) struct NesRgb {
    source: LogicalSize,
}

impl NesRgb {
    pub(crate) fn new(source: LogicalSize) -> Self {
        Self { source }
    }
}

impl FilterUnit for NesRgb {
    type Input = u8;
    type Output = RGB;

    fn push<F: FnMut(Self::Output)>(&mut self, value: Self::Input, next_func: &mut F) {
        next_func(RGB::from(PALETTE[usize::from(value)]))
    }

    fn source_logical_size(&self) -> LogicalSize {
        self.source
    }

    fn source_physical_size(&self) -> PhysicalSize {
        PhysicalSize::from(self.source)
    }

    fn eval_logical_size(source: LogicalSize) -> LogicalSize {
        source
    }

    fn eval_physical_size(source: PhysicalSize) -> PhysicalSize {
        PhysicalSize {
            width: source.width * 8.0 / 7.0,
            height: source.height,
        }
    }
}
