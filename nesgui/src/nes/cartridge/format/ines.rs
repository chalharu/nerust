// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::error::CartridgeError;
use super::CartridgeData;
use core::cmp;
use nes::MirrorMode;

pub(crate) fn read_ines<I: Iterator<Item = u8>>(
    input: &mut I,
) -> Result<CartridgeData, CartridgeError> {
    let headers = input.take(12).collect::<Vec<_>>();
    if headers.len() != 12 {
        return Err(CartridgeError::UnexpectedEof);
    }
    let prom_length = usize::from(headers[0]) * 0x4000;
    let crom_length = usize::from(headers[1]) * 0x2000;
    let flags1 = headers[2];
    let flags2 = headers[3];
    let ram_length = cmp::max(usize::from(headers[4]), 1) * 0x2000;

    let mapper_type = (flags1 >> 4) | (flags2 & 0xf0);
    let mirror_type = (flags1 & 1) | ((flags1 >> 2) & 2);
    let has_battery = (flags1 & 2) == 2;
    let has_trainer = (flags1 & 4) == 4;
    let _trainer = if has_trainer {
        let tmp = input.take(512).collect::<Vec<_>>();
        if tmp.len() != 512 {
            return Err(CartridgeError::UnexpectedEof);
        }
        tmp
    } else {
        Vec::new()
    };
    let prog_rom = input.take(prom_length).collect::<Vec<_>>();
    if prog_rom.len() != prom_length {
        return Err(CartridgeError::UnexpectedEof);
    }
    let char_rom = if crom_length != 0 {
        let tmp = input.take(crom_length).collect::<Vec<_>>();
        if tmp.len() != crom_length {
            return Err(CartridgeError::UnexpectedEof);
        }
        tmp
    } else {
        vec![0; 8192]
    };
    let mirror_mode =
        try!(MirrorMode::try_from(mirror_type).map_err(|_| CartridgeError::DataError));
    Ok(CartridgeData {
        prog_rom,
        char_rom,
        sram: vec![0; ram_length],
        mapper_type,
        mirror_mode,
        has_battery,
    })
}
