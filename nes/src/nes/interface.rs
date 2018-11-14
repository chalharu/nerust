// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub trait Screen {
    fn push(&mut self, palette: u8);
    fn render(&mut self);
}

pub trait Speaker {
    fn push(&mut self, data: i16);
}
