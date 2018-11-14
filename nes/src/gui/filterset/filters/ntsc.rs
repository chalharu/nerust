// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{FilterUnit, LogicalSize, PhysicalSize};
use crate::gui::RGB;

impl From<nes_ntsc::RGB> for RGB {
    fn from(value: nes_ntsc::RGB) -> RGB {
        RGB {
            red: value.red,
            green: value.green,
            blue: value.green,
        }
    }
}

pub struct NesNtsc {
    ntsc: nes_ntsc::NesNtsc,
    source: LogicalSize,
}

pub enum Setup {
    Composite,
    SVideo,
    RGB,
}

impl NesNtsc {
    pub fn new(setup: &Setup, source: LogicalSize) -> Self {
        match *setup {
            Setup::Composite => Self::composite(source),
            Setup::SVideo => Self::svideo(source),
            Setup::RGB => Self::rgb(source),
        }
    }

    pub fn composite(source: LogicalSize) -> Self {
        Self {
            ntsc: nes_ntsc::NesNtsc::new(&nes_ntsc::Setup::Composite, source.width),
            source,
        }
    }

    pub fn svideo(source: LogicalSize) -> Self {
        Self {
            ntsc: nes_ntsc::NesNtsc::new(&nes_ntsc::Setup::SVideo, source.width),
            source,
        }
    }

    pub fn rgb(source: LogicalSize) -> Self {
        Self {
            ntsc: nes_ntsc::NesNtsc::new(&nes_ntsc::Setup::RGB, source.width),
            source,
        }
    }
}

impl FilterUnit for NesNtsc {
    type Input = u8;
    type Output = RGB;

    fn push<F: FnMut(Self::Output)>(&mut self, value: Self::Input, next_func: &mut F) {
        self.ntsc.push(value, &mut |x| next_func(RGB::from(x)));
    }

    fn source_logical_size(&self) -> LogicalSize {
        self.source
    }

    fn source_physical_size(&self) -> PhysicalSize {
        PhysicalSize::from(self.source)
    }

    fn eval_logical_size(source: LogicalSize) -> LogicalSize {
        LogicalSize {
            width: nes_ntsc::NesNtsc::output_width(source.width),
            height: source.height,
        }
    }

    fn eval_physical_size(source: PhysicalSize) -> PhysicalSize {
        PhysicalSize {
            width: nes_ntsc::NesNtsc::output_width(source.width as usize) as f32,
            height: source.height * 2 as f32,
        }
    }
}
