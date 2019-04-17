// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub trait Sound {
    fn start(&mut self);
    fn pause(&mut self);
}

pub trait MixerInput {
    fn push(&mut self, data: f32); // 0.0 ~ 1.0
}
