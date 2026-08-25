use crate::{
    cartridge::Cartridge,
    cartridge_header::CartridgeHeader,
    cartridge_mbc,
    cpu_core::{GbcModel, Lr35902Cpu},
    memory::GbcMemoryBus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareModel {
    Dmg0,
    Dmg,
    CgbC,
    CgbD,
    Agb,
}

pub struct GbcSystem {
    pub cpu: Lr35902Cpu,
    pub bus: GbcMemoryBus,
}

impl GbcSystem {
    pub fn from_rom_without_boot_rom(model: HardwareModel, rom_bytes: Vec<u8>) -> Option<Self> {
        let header = CartridgeHeader::parse(&rom_bytes)?;
        let rom_is_cgb = header.cgb_flag & 0x80 != 0;
        let font_bank1 = if rom_bytes.len() > 0x4000 {
            Some(rom_bytes[0x4000..rom_bytes.len().min(0x4800)].to_vec())
        } else {
            None
        };
        let mbc = cartridge_mbc::create_mbc(&header, rom_bytes, None);

        let hw_is_cgb = matches!(
            model,
            HardwareModel::CgbC | HardwareModel::CgbD | HardwareModel::Agb
        );
        let mut bus = GbcMemoryBus::new([0; 0x100], false);
        bus.set_cartridge(Cartridge::new(mbc));
        if let Some(font) = font_bank1 {
            bus.load_font_tiles(&font);
        }
        bus.set_cgb_mode(hw_is_cgb);
        bus.set_cgb_revision_d(matches!(model, HardwareModel::CgbD | HardwareModel::Agb));
        bus.set_cgb_game(hw_is_cgb && rom_is_cgb);
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

    fn minimal_rom(cgb: bool) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0143] = if cgb { 0x80 } else { 0 };
        rom[0x0144] = b'0';
        rom[0x0145] = b'1';
        rom[0x0147] = 0;
        rom[0x0148] = 0;
        rom[0x0149] = 0;
        rom[0x014B] = 0x33;
        rom
    }

    #[test]
    fn rejects_rom_without_header() {
        assert!(GbcSystem::from_rom_without_boot_rom(HardwareModel::Dmg, vec![]).is_none());
    }

    #[test]
    fn initializes_dmg_post_boot_counter() {
        let system =
            GbcSystem::from_rom_without_boot_rom(HardwareModel::Dmg, minimal_rom(false)).unwrap();
        assert_eq!(system.bus.read(0xFF04), 0xAB);
        assert_eq!(system.cpu.registers().pc(), 0x0100);
    }

    #[test]
    fn initializes_cgb_dmg_compatibility_registers() {
        let system =
            GbcSystem::from_rom_without_boot_rom(HardwareModel::CgbD, minimal_rom(false)).unwrap();
        let registers = system.cpu.registers();
        assert_eq!(registers.d(), 0x00);
        assert_eq!(registers.e(), 0x08);
        assert_eq!(registers.h(), 0x00);
        assert_eq!(registers.l(), 0x7C);
    }
}
