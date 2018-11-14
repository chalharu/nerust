// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod filters;

use super::{LogicalSize, PhysicalSize, RGB};

pub trait NesFilter {
    fn push<'a>(&'a mut self, value: u8, next_func: Box<dyn FnMut(RGB) + 'a>);

    fn logical_size(&self) -> LogicalSize;
    fn physical_size(&self) -> PhysicalSize;
}

impl<F: filters::FilterUnit<Input = u8, Output = RGB>> NesFilter for F {
    fn push<'a>(&'a mut self, value: u8, mut next_func: Box<dyn FnMut(RGB) + 'a>) {
        filters::FilterUnit::push(self, value, &mut |x| next_func(x))
    }

    fn logical_size(&self) -> LogicalSize {
        filters::FilterUnit::logical_size(self)
    }

    fn physical_size(&self) -> PhysicalSize {
        filters::FilterUnit::physical_size(self)
    }
}

pub enum FilterType {
    None,
    NtscRGB,
    NtscComposite,
    NtscSVideo,
}

impl FilterType {
    pub fn generate(&self, size: LogicalSize) -> Box<dyn NesFilter> {
        match *self {
            FilterType::None => Box::new(filters::rgb::NesRgb::new(size)),
            FilterType::NtscRGB => Box::new(filters::ntsc::NesNtsc::rgb(size)),
            FilterType::NtscComposite => Box::new(filters::ntsc::NesNtsc::composite(size)),
            FilterType::NtscSVideo => Box::new(filters::ntsc::NesNtsc::svideo(size)),
        }
    }
}
