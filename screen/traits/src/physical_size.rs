// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::logical_size::LogicalSize;

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct PhysicalSize {
    pub width: f32,
    pub height: f32,
}

impl From<LogicalSize> for PhysicalSize {
    fn from(value: LogicalSize) -> PhysicalSize {
        PhysicalSize {
            width: value.width as f32,
            height: value.height as f32,
        }
    }
}
