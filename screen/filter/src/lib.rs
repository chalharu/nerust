// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod filters;

use nerust_screen_traits::{LogicalSize, PhysicalSize, RGB};

pub trait NesFilter {
    fn push(&mut self, value: u8, filter_func: &mut FilterFunc);

    fn logical_size(&self) -> LogicalSize;
    fn physical_size(&self) -> PhysicalSize;
}

pub trait FilterFunc {
    fn filter_func(&mut self, value: RGB);
}

impl<F: filters::FilterUnit<Input = u8, Output = RGB>> NesFilter for F {
    fn push(&mut self, value: u8, filter_func: &mut FilterFunc) {
        filters::FilterUnit::push(self, value, &mut |x| filter_func.filter_func(x))
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
