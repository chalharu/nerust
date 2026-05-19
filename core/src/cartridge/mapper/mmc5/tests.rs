use super::{ChrBankSet, Mmc5};
use crate::cart_device::Cartridge;
use crate::cpu::interrupt::Interrupt;
use crate::mapper::Mapper;
use crate::ppu::Core as PpuCore;
use crate::ppu_memory_access::PpuReadAccess;
use crate::{CartridgeData, CartridgeDataParts, MirrorMode, RomFormat};

fn test_data() -> CartridgeData {
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::Nes20,
        prog_rom: (0..0x20000).map(|i| (i / 0x2000) as u8).collect(),
        char_rom: (0..0x40000).map(|i| (i / 0x400) as u8).collect(),
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 5,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

#[test]
fn ppudata_uses_last_written_chr_bank_set_in_split_mode() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5101, 0x03, &mut Interrupt::new());
    mapper.notify_ppu_ctrl(0x20);
    mapper.notify_ppu_mask(0x18);

    mapper.write_expansion(0x5120, 0x02, &mut Interrupt::new());
    mapper.write_expansion(0x5128, 0x07, &mut Interrupt::new());

    assert_eq!(mapper.last_chr_bank_set, ChrBankSet::Background);
    assert_eq!(
        mapper
            .read_ppu_pattern(0x0000, PpuReadAccess::CpuData, &mut Interrupt::new())
            .data,
        0x07
    );
}

#[test]
fn extended_attributes_override_background_palette_and_chr_bank() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.notify_ppu_mask(0x18);
    mapper.write_expansion(0x5104, 0x01, &mut Interrupt::new());
    mapper.write_expansion(0x5130, 0x01, &mut Interrupt::new());
    mapper.exram[0] = 0b10_000011;

    let mut ciram = vec![0; 0x800];
    let _ = mapper.read_ppu_nametable(0x2000, PpuReadAccess::BackgroundNameTable, &mut ciram);

    assert_eq!(
        mapper
            .read_ppu_nametable(0x23C0, PpuReadAccess::BackgroundAttribute, &mut ciram)
            .data,
        0xAA
    );
    assert_eq!(
        mapper
            .read_ppu_pattern(
                0x0000,
                PpuReadAccess::BackgroundPattern,
                &mut Interrupt::new()
            )
            .data,
        0x0C
    );
}

#[test]
fn fill_mode_supplies_nametable_and_attribute_bytes() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    Mapper::write_expansion(&mut mapper, 0x5105, 0xFF, &mut Interrupt::new());
    Mapper::write_expansion(&mut mapper, 0x5106, 0x25, &mut Interrupt::new());
    Mapper::write_expansion(&mut mapper, 0x5107, 0x03, &mut Interrupt::new());
    let mut ciram = vec![0; 0x800];

    assert_eq!(
        mapper
            .read_ppu_nametable(0x2000, PpuReadAccess::CpuData, &mut ciram)
            .data,
        0x25
    );
    assert_eq!(
        mapper
            .read_ppu_nametable(0x23C0, PpuReadAccess::CpuData, &mut ciram)
            .data,
        0xFF
    );
}

#[test]
fn exram_can_be_executed_from_expansion_space() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
    mapper.write_expansion(0x5C00, 0xA9, &mut Interrupt::new());
    mapper.write_expansion(0x5C01, 0x5A, &mut Interrupt::new());

    assert_eq!(mapper.read_expansion(0x5C00).data, 0xA9);
    assert_eq!(mapper.read_expansion(0x5C01).data, 0x5A);
}

#[test]
fn mirrored_ppuctrl_writes_do_not_toggle_sprite_size() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut ppu = PpuCore::new();
    let mut interrupt = Interrupt::new();

    ppu.write_register(0x2008, 0x20, &mut mapper, &mut interrupt);
    assert!(!mapper.sprite_size_16);

    ppu.write_register(0x2000, 0x20, &mut mapper, &mut interrupt);
    assert!(mapper.sprite_size_16);
}

#[test]
fn exram_mode_zero_rejects_cpu_writes() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.write_expansion(0x5C00, 0xA9, &mut Interrupt::new());

    assert_eq!(mapper.read_expansion(0x5C00).mask, 0);

    mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
    assert_eq!(mapper.read_expansion(0x5C00).data, 0x00);
}
