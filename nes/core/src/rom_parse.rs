use std::cmp;

use crate::cartridge_data_parts::CartridgeDataParts;
use crate::cartridge_error::CartridgeError;
use crate::cartridge_rom::CartridgeData;
use crate::mirror::MirrorMode;
use crate::rom_format::RomFormat;

/// Raw ROM バイト列をパースして CartridgeData を生成する。
/// iNES または NES 2.0 を自動判別する。
type RomChunks = (Vec<u8>, Vec<u8>, Vec<u8>);

fn extract_chunks(
    data: &[u8],
    prom_length: usize,
    crom_length: usize,
    has_trainer: bool,
) -> Result<RomChunks, CartridgeError> {
    let mut offset = 16;
    let trainer = if has_trainer {
        let end = offset + 512;
        if end > data.len() {
            return Err(CartridgeError::UnexpectedEof);
        }
        offset = end;
        data[16..end].to_vec()
    } else {
        Vec::new()
    };

    let prog_end = offset + prom_length;
    if prog_end > data.len() {
        return Err(CartridgeError::UnexpectedEof);
    }
    let prog_rom = data[offset..prog_end].to_vec();
    offset = prog_end;

    let char_rom = if crom_length != 0 {
        let chr_end = offset + crom_length;
        if chr_end > data.len() {
            return Err(CartridgeError::UnexpectedEof);
        }
        data[offset..chr_end].to_vec()
    } else {
        Vec::new()
    };

    Ok((trainer, prog_rom, char_rom))
}

pub fn parse_rom(data: &[u8]) -> Result<CartridgeData, CartridgeError> {
    if data.len() < 16 {
        return Err(CartridgeError::UnexpectedEof);
    }
    if data[0] != 0x4E || data[1] != 0x45 || data[2] != 0x53 || data[3] != 0x1A {
        return Err(CartridgeError::DataError);
    }

    if data[7] & 0x0C == 0x08 {
        parse_nes20(data)
    } else {
        parse_ines(data)
    }
}

fn parse_ines(data: &[u8]) -> Result<CartridgeData, CartridgeError> {
    let prom_length = usize::from(data[4]) * 0x4000;
    let crom_length = usize::from(data[5]) * 0x2000;
    let flags1 = data[6];
    let flags2 = data[7];
    let pram_length = cmp::max(usize::from(data[8]), 1) * 0x2000;

    let mapper_type = u16::from((flags1 >> 4) | (flags2 & 0xf0));
    let mirror_bits = (flags1 & 1) | ((flags1 >> 2) & 2);
    let mirror_mode = MirrorMode::try_from(mirror_bits).map_err(|_| CartridgeError::DataError)?;
    let has_battery = (flags1 & 2) == 2;
    let has_trainer = (flags1 & 4) == 4;

    let (trainer, prog_rom, char_rom) =
        extract_chunks(data, prom_length, crom_length, has_trainer)?;
    let vram_length = if crom_length != 0 { 0 } else { 0x2000 };

    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom,
        char_rom,
        pram_length,
        save_pram_length: 0,
        vram_length,
        save_vram_length: 0,
        mapper_type,
        mirror_mode,
        has_battery,
        sub_mapper_type: 0,
        trainer,
    })
}

fn parse_nes20(data: &[u8]) -> Result<CartridgeData, CartridgeError> {
    let upper_rom_size = usize::from(data[9]);
    let prom_length = (usize::from(data[4]) | ((upper_rom_size & 0x0F) << 8)) * 0x4000;
    let crom_length = (usize::from(data[5]) | ((upper_rom_size & 0xF0) << 4)) * 0x2000;
    let flags1 = data[6];
    let flags2 = data[7];
    let mapper_variant = data[8];
    let pram_length_data = usize::from(data[10]);
    let vram_length_data = usize::from(data[11]);
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
    let mirror_bits = (flags1 & 1) | ((flags1 >> 2) & 2);
    let mirror_mode = MirrorMode::try_from(mirror_bits).map_err(|_| CartridgeError::DataError)?;
    let has_battery = (flags1 & 2) == 2;
    let has_trainer = (flags1 & 4) == 4;

    let (trainer, prog_rom, char_rom) =
        extract_chunks(data, prom_length, crom_length, has_trainer)?;

    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::Nes20,
        prog_rom,
        char_rom,
        pram_length,
        save_pram_length,
        vram_length,
        save_vram_length,
        mapper_type,
        mirror_mode,
        has_battery,
        sub_mapper_type,
        trainer,
    })
}
