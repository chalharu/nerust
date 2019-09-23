// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod logical_size;
mod physical_size;
mod rgb;

pub use logical_size::LogicalSize;
pub use physical_size::PhysicalSize;
pub use rgb::RGB;

pub trait Screen {
    fn push(&mut self, palette: u8);
    fn render(&mut self);
}
