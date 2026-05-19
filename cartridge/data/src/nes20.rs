// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{CartridgeParseError, cartridge_data, validate_mirror_mode};
use nerust_core::{CartridgeData, CartridgeDataParts, RomFormat};

pub(crate) fn read_nes20<I: Iterator<Item = u8>>(
    headers: &[u8],
    input: &mut I,
) -> Result<CartridgeData, CartridgeParseError> {
    let upper_rom_size = usize::from(headers[9]);
    let prom_length = (usize::from(headers[4]) | ((upper_rom_size & 0x0F) << 8)) * 0x4000;
    let crom_length = (usize::from(headers[5]) | ((upper_rom_size & 0xF0) << 4)) * 0x2000;
    let flags1 = headers[6];
    let flags2 = headers[7];
    let mapper_variant = headers[8];
    let pram_length_data = usize::from(headers[10]);
    let vram_length_data = usize::from(headers[11]);
    let pram_length_shift = pram_length_data & 0x0F;
    let save_pram_length_shift = pram_length_data >> 4;
    let vram_length_shift = vram_length_data & 0x0F;
    let save_vram_length_shift = vram_length_data >> 4;

    let pram_length = if pram_length_shift == 0 {
        0
    } else {
        1 << (6 + pram_length_shift)
    };
    let save_pram_length = if save_pram_length_shift == 0 {
        0
    } else {
        1 << (6 + save_pram_length_shift)
    };
    let vram_length = if vram_length_shift == 0 {
        if crom_length == 0 && save_vram_length_shift == 0 {
            0x2000
        } else {
            0
        }
    } else {
        1 << (6 + vram_length_shift)
    };
    let save_vram_length = if save_vram_length_shift == 0 {
        0
    } else {
        1 << (6 + save_vram_length_shift)
    };

    let mapper_type =
        u16::from(flags1 >> 4) | u16::from(flags2 & 0xf0) | (u16::from(mapper_variant & 0x0F) << 8);
    let sub_mapper_type = mapper_variant >> 4;
    let mirror_mode = validate_mirror_mode((flags1 & 1) | ((flags1 >> 2) & 2))?;
    let has_battery = (flags1 & 2) == 2;
    let has_trainer = (flags1 & 4) == 4;
    let trainer = if has_trainer {
        let tmp = input.take(512).collect::<Vec<_>>();
        if tmp.len() != 512 {
            return Err(CartridgeParseError::UnexpectedEof);
        }
        tmp
    } else {
        Vec::new()
    };
    let prog_rom = input.take(prom_length).collect::<Vec<_>>();
    if prog_rom.len() != prom_length {
        return Err(CartridgeParseError::UnexpectedEof);
    }
    let char_rom = if crom_length != 0 {
        let tmp = input.take(crom_length).collect::<Vec<_>>();
        if tmp.len() != crom_length {
            return Err(CartridgeParseError::UnexpectedEof);
        }
        tmp
    } else {
        Vec::new()
    };
    cartridge_data(CartridgeDataParts {
        format: RomFormat::Nes20,
        prog_rom,
        char_rom,
        mapper_type,
        mirror_mode,
        has_battery,
        sub_mapper_type,
        pram_length,
        save_pram_length,
        vram_length,
        save_vram_length,
        trainer,
    })
}
