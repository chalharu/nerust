// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[repr(packed)]
#[derive(Debug, Copy, Clone)]
pub(crate) struct Vec2D {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Vec2D {
    pub(crate) fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
