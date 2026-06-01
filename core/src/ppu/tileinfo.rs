// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct TileInfo {
    pub(crate) low_byte: u8,
    pub(crate) high_byte: u8,
    pub(crate) palette_offset: u8,
    pub(crate) tile_addr: u16,
}

impl TileInfo {
    pub(crate) fn new() -> Self {
        Self {
            low_byte: 0,
            high_byte: 0,
            palette_offset: 0,
            tile_addr: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.low_byte = 0;
        self.high_byte = 0;
        self.palette_offset = 0;
        self.tile_addr = 0;
    }
}
