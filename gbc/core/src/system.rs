use crate::{
    cartridge::Cartridge,
    cartridge_descriptor::{CartridgeDescriptor, detect_cartridge},
    cartridge_mbc,
    cpu_core::{GbcModel, Lr35902Cpu},
    memory::GbcMemoryBus,
};

pub use nerust_gbc_settings::HardwareModel;

pub struct GbcSystem {
    pub cpu: Lr35902Cpu,
    pub bus: GbcMemoryBus,
}

impl GbcSystem {
    pub fn from_rom(model: HardwareModel, rom_bytes: Vec<u8>) -> Option<Self> {
        let descriptor = detect_cartridge(&rom_bytes)?;
        Self::from_descriptor(model, rom_bytes, &descriptor)
    }

    pub fn from_descriptor(
        model: HardwareModel,
        rom_bytes: Vec<u8>,
        descriptor: &CartridgeDescriptor,
    ) -> Option<Self> {
        let header = &descriptor.header;
        let rom_is_cgb = header.cgb_flag & 0x80 != 0;
        let font_start = descriptor.initial_romx_bank.checked_mul(0x4000)?;
        let font_bank1 = if rom_bytes.len() > font_start {
            Some(rom_bytes[font_start..rom_bytes.len().min(font_start + 0x800)].to_vec())
        } else {
            None
        };
        let compatibility_palettes = (!rom_is_cgb)
            .then(|| crate::compatibility_palette::select(&rom_bytes[descriptor.header_offset..]));
        let mbc = cartridge_mbc::create_mbc_from_descriptor(descriptor, rom_bytes, None);

        let hw_is_cgb = matches!(
            model,
            HardwareModel::CgbC | HardwareModel::CgbD | HardwareModel::Agb
        );
        let mut bus = GbcMemoryBus::new();
        bus.set_cartridge(Cartridge::new(mbc));
        if let Some(font) = font_bank1 {
            bus.load_font_tiles(&font);
        }
        bus.set_cgb_mode(hw_is_cgb);
        bus.set_cgb_revision_d(matches!(model, HardwareModel::CgbD | HardwareModel::Agb));
        bus.set_cgb_game(hw_is_cgb && rom_is_cgb);
        if hw_is_cgb && let Some(palettes) = compatibility_palettes {
            bus.set_dmg_compatibility_palettes(palettes);
        }
        bus.set_boot_counter(match model {
            HardwareModel::Dmg0 => 0x182F,
            HardwareModel::Dmg => 0xABCB,
            HardwareModel::CgbC | HardwareModel::CgbD | HardwareModel::Agb => 0x2677,
        });
        if model == HardwareModel::Dmg0 {
            bus.set_ppu_frame_phase(66220);
        }
        bus.set_post_boot_io(hw_is_cgb);
        bus.set_post_boot_key1(hw_is_cgb && rom_is_cgb);

        let cpu_model = match model {
            HardwareModel::Dmg0 => GbcModel::Dmg0,
            HardwareModel::Dmg => GbcModel::Dmg,
            HardwareModel::CgbC | HardwareModel::CgbD => GbcModel::Cgb,
            HardwareModel::Agb => GbcModel::Agb,
        };
        let mut cpu = Lr35902Cpu::with_model(cpu_model);
        if hw_is_cgb && !rom_is_cgb {
            cpu.set_cgb_dmg_mode_registers();
        }

        Some(Self { cpu, bus })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banked_rom(bank_count: usize) -> Vec<u8> {
        let mut rom = vec![0; bank_count * 0x4000];
        for (bank, bytes) in rom.chunks_exact_mut(0x4000).enumerate() {
            bytes.fill(bank as u8);
        }
        rom
    }

    fn minimal_rom(cgb: bool) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0143] = if cgb { 0x80 } else { 0 };
        rom[0x0144] = b'0';
        rom[0x0145] = b'1';
        rom[0x0147] = 0;
        rom[0x0148] = 0;
        rom[0x0149] = 0;
        rom[0x014B] = 0x33;
        crate::cartridge_header::finalize_test_rom(&mut rom);
        rom
    }

    #[test]
    fn rejects_rom_without_header() {
        assert!(GbcSystem::from_rom(HardwareModel::Dmg, vec![]).is_none());
    }

    #[test]
    fn initializes_dmg_post_boot_counter() {
        let system = GbcSystem::from_rom(HardwareModel::Dmg, minimal_rom(false)).unwrap();
        assert_eq!(system.bus.read(0xFF04), 0xAB);
        assert_eq!(system.cpu.registers().pc(), 0x0100);
    }

    #[test]
    fn initializes_cgb_dmg_compatibility_registers() {
        let system = GbcSystem::from_rom(HardwareModel::CgbD, minimal_rom(false)).unwrap();
        let registers = system.cpu.registers();
        assert_eq!(registers.d(), 0x00);
        assert_eq!(registers.e(), 0x08);
        assert_eq!(registers.h(), 0x00);
        assert_eq!(registers.l(), 0x7C);
    }

    #[test]
    fn loads_mmm01_from_trailing_header_and_starts_in_menu_banks() {
        let mut rom = banked_rom(8);
        let menu_base = rom.len() - 0x8000;
        rom[menu_base + 0x0100..menu_base + 0x0150].fill(0);
        rom[menu_base + 0x0147] = 0x0B;
        rom[menu_base + 0x0148] = 2;
        crate::cartridge_header::finalize_test_rom(&mut rom[menu_base..]);

        let system = GbcSystem::from_rom(HardwareModel::Dmg, rom).unwrap();
        assert_eq!(system.bus.read(0), 6);
        assert_eq!(system.bus.read(0x4000), 7);
    }

    #[test]
    fn detected_wisdom_tree_mapper_switches_the_full_rom_window() {
        let mut rom = banked_rom(8);
        rom[0x0147] = 0;
        rom[0x0148] = 0;
        rom[0x00F0..0x0100].fill(0);
        rom[0x0134..0x014C].fill(0);
        rom[0x0300..0x030B].copy_from_slice(b"WISDOM TREE");
        crate::cartridge_header::finalize_test_rom(&mut rom);

        let mut system = GbcSystem::from_rom(HardwareModel::Dmg, rom).unwrap();
        system.bus.write(2, 0);
        assert_eq!(system.bus.read(0), 4);
        assert_eq!(system.bus.read(0x4000), 5);
    }
}
