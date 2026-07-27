/// Post-BIOS register state applied when boot_rom_enabled = false.
///
/// When BIOS is skipped, the console core writes these values into the
/// CPU registers and PPU registers instead of running the real boot ROM.
#[derive(Debug, Clone, Copy)]
pub struct PostBiosState {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ppu_lcdc: u8,
    pub ppu_bgp: u8,
    pub ppu_obp0: u8,
    pub ppu_obp1: u8,
    pub ppu_ly: u8,
    pub boot_rom_mapped: bool,
}

pub fn post_bios_state(is_cgb: bool) -> PostBiosState {
    let mut state = PostBiosState {
        a: 0x01,
        f: 0xB0,
        b: 0x00,
        c: 0x13,
        d: 0x00,
        e: 0xD8,
        h: 0x01,
        l: 0x4D,
        sp: 0xFFFE,
        pc: 0x0100,
        ppu_lcdc: 0x91,
        ppu_bgp: 0xFC,
        ppu_obp0: 0xFF,
        ppu_obp1: 0xFF,
        ppu_ly: 0x00,
        boot_rom_mapped: false,
    };
    if is_cgb {
        state.a = 0x11;
    }
    state
}

/// DMG boot ROM (256 bytes).
///
/// Not embedded in the binary; loaded from a file at runtime when
/// `boot_rom_enabled` is true.
pub const BOOT_ROM_SIZE: usize = 0x100;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmg_post_bios_a_is_0x01() {
        let state = post_bios_state(false);
        assert_eq!(state.a, 0x01);
    }

    #[test]
    fn cgb_post_bios_a_is_0x11() {
        let state = post_bios_state(true);
        assert_eq!(state.a, 0x11);
    }

    #[test]
    fn post_bios_pc_is_0x0100() {
        for cgb in [false, true] {
            assert_eq!(post_bios_state(cgb).pc, 0x0100);
        }
    }

    #[test]
    fn post_bios_sp_is_0xfffe() {
        assert_eq!(post_bios_state(false).sp, 0xFFFE);
    }

    #[test]
    fn post_bios_boot_rom_is_unmapped() {
        assert!(!post_bios_state(false).boot_rom_mapped);
        assert!(!post_bios_state(true).boot_rom_mapped);
    }

    #[test]
    fn post_bios_ppu_registers_match_bios_end_state() {
        let state = post_bios_state(false);
        assert_eq!(state.ppu_lcdc, 0x91);
        assert_eq!(state.ppu_bgp, 0xFC);
        assert_eq!(state.ppu_obp0, 0xFF);
        assert_eq!(state.ppu_obp1, 0xFF);
        assert_eq!(state.ppu_ly, 0x00);
    }
}
