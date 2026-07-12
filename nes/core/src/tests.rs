#![cfg(test)]

use crate::{
    Core, cartridge_data_parts::CartridgeDataParts, cartridge_rom::CartridgeData,
    mirror::MirrorMode, rom_format::RomFormat,
};

#[test]
fn inspect_cartridge_reads_ines_metadata() {
    let cartridge_data = CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom: vec![0; 0x8000],
        char_rom: vec![0; 0x2000],
        pram_length: 0x2000,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 4,
        mirror_mode: MirrorMode::Vertical,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid");

    let info = Core::inspect_cartridge(&cartridge_data, 16 + 0x8000 + 0x2000)
        .expect("rom info should inspect");

    assert_eq!(info.format, RomFormat::INes);
    assert_eq!(info.mapper_type, 4);
    assert_eq!(info.sub_mapper_type, 0);
    assert_eq!(info.mirror_mode, MirrorMode::Vertical);
    assert!(!info.has_battery);
    assert_eq!(info.trainer_len, 0);
    assert_eq!(info.prg_rom_len, 0x8000);
    assert_eq!(info.chr_rom_len, 0x2000);
    assert_eq!(info.prg_ram_len, 0x2000);
    assert_eq!(info.save_prg_ram_len, 0);
    assert_eq!(info.chr_ram_len, 0);
    assert_eq!(info.save_chr_ram_len, 0);
    assert_eq!(info.raw_file_len, 16 + 0x8000 + 0x2000);
    assert_eq!(info.body_len, 0x8000 + 0x2000);
}

#[test]
fn inspect_cartridge_reports_effective_legacy_battery_save_ram_length() {
    let cartridge_data = CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom: vec![0; 0x20000],
        char_rom: vec![0; 0x2000],
        pram_length: 0x2000,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 1,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: true,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid");

    let info = Core::inspect_cartridge(&cartridge_data, 16 + 0x20000 + 0x2000).expect("inspect");

    assert_eq!(info.prg_ram_len, 0x2000);
    assert_eq!(info.save_prg_ram_len, 0x2000);
}

#[test]
fn rejects_too_small_program_rom() {
    let result = CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom: vec![0; 0x2000],
        char_rom: vec![0; 0x2000],
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 0,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    });

    assert!(result.is_err());
}
