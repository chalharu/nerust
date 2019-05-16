// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub enum MirrorMode {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
}

impl MirrorMode {
    pub fn try_from<'a>(mode: u8) -> Result<MirrorMode, &'a str> {
        match mode {
            0 => Ok(MirrorMode::Horizontal),
            1 => Ok(MirrorMode::Vertical),
            2 => Ok(MirrorMode::Single0),
            3 => Ok(MirrorMode::Single1),
            4 => Ok(MirrorMode::Four),
            _ => Err("parse error"),
        }
    }

    fn mirror_lut(self) -> [u8; 4] {
        match self {
            MirrorMode::Horizontal => [0, 0, 1, 1],
            MirrorMode::Vertical => [0, 1, 0, 1],
            MirrorMode::Single0 => [0, 0, 0, 0],
            MirrorMode::Single1 => [1, 1, 1, 1],
            MirrorMode::Four => [0, 1, 2, 3],
        }
    }

    pub fn mirror_address(self, address: usize) -> usize {
        let vram_address = address & 0x0FFF;
        let table = vram_address >> 10;
        let offset = vram_address & 0x3FF;
        0x2000 + (usize::from(self.mirror_lut()[table]) << 10) + offset
    }
}
