// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MirrorMode {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
    Custom([u8; 4]),
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
}
