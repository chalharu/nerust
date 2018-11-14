// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{FilterUnit, LogicalSize, PhysicalSize};
use crate::gui::RGB;

const PALETTE: [u32; 64] = [
    0x666666, 0x002A88, 0x1412A7, 0x3B00A4, 0x5C007E, 0x6E0040, 0x6C0600, 0x561D00, 0x333500,
    0x0B4800, 0x005200, 0x004F08, 0x00404D, 0x000000, 0x000000, 0x000000, 0xADADAD, 0x155FD9,
    0x4240FF, 0x7527FE, 0xA01ACC, 0xB71E7B, 0xB53120, 0x994E00, 0x6B6D00, 0x388700, 0x0C9300,
    0x008F32, 0x007C8D, 0x000000, 0x000000, 0x000000, 0xFFFEFF, 0x64B0FF, 0x9290FF, 0xC676FF,
    0xF36AFF, 0xFE6ECC, 0xFE8170, 0xEA9E22, 0xBCBE00, 0x88D800, 0x5CE430, 0x45E082, 0x48CDDE,
    0x4F4F4F, 0x000000, 0x000000, 0xFFFEFF, 0xC0DFFF, 0xD3D2FF, 0xE8C8FF, 0xFBC2FF, 0xFEC4EA,
    0xFECCC5, 0xF7D8A5, 0xE4E594, 0xCFEF96, 0xBDF4AB, 0xB3F3CC, 0xB5EBF2, 0xB8B8B8, 0x000000,
    0x000000,
];

pub struct NesRgb {
    source: LogicalSize,
}

impl NesRgb {
    pub fn new(source: LogicalSize) -> Self {
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
