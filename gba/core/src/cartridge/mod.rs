pub mod gpio;
pub mod header;
pub mod rtc;
pub mod save;
pub mod solar;

use self::gpio::Gpio;
use self::header::GbaHeader;
use self::save::helpers::read_slice;
use self::save::{SaveBackend, SaveType, create_save_backend, detect_save_type};
use crate::rom_identity::SaveTypeSer;

#[derive(Debug)]
pub struct Cartridge {
    pub header: GbaHeader,
    pub rom: Vec<u8>,
    pub save: Box<dyn SaveBackend>,
    pub gpio: Gpio,
}

/// Phase 10 wire state: the backend runtime blob plus GPIO/RTC/Solar
/// runtime. ROM bytes and the parsed header rebuild from the retained ROM.
/// Also reused as the reset-transfer state (`CartridgeResetState`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CartridgeState {
    save_kind: SaveTypeSer,
    save_blob: serde_bytes::ByteBuf,
    gpio: Gpio,
}

impl CartridgeState {
    pub(crate) fn validate_against(&self, save_type: SaveType) -> Result<(), String> {
        if self.save_kind != SaveTypeSer::from(save_type) {
            return Err(format!(
                "cartridge: save kind mismatch: wire={:?} actual={save_type:?}",
                self.save_kind
            ));
        }
        Ok(())
    }
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Option<Self> {
        let header = GbaHeader::parse(&rom)?;
        let save_type = detect_save_type(&rom);
        let save = create_save_backend(save_type);
        Some(Self {
            header,
            rom,
            save,
            gpio: Gpio::new(),
        })
    }

    pub fn read_rom(&self, addr: u32, width: u8) -> u32 {
        let len = self.rom.len();
        if len == 0 {
            return oob_pattern(addr, width);
        }
        let base = 0x08000000;
        let raw_off = ((addr - base) & 0x01FF_FFFF) as usize;
        // Unaligned loads read from the aligned address (halfword/word align down).
        // Byte loads use the exact address.
        let aligned_off = match width {
            4 => raw_off & !3,
            2 => raw_off & !1,
            _ => raw_off,
        };
        // Beyond the cartridge size the GamePak bus floats: open bus,
        // not size mirroring (mgba-suite "ROM out-of-bounds load" pins
        // the (address/2) pattern for CPU/DMA/CpuSet alike).
        if aligned_off >= len {
            return oob_pattern(addr, width);
        }
        let off = if len.is_power_of_two() {
            aligned_off & (len - 1)
        } else {
            aligned_off % len
        };
        read_slice(&self.rom, off, width)
    }

    pub fn read_sram(&self, addr: u32, width: u8) -> u32 {
        self.save.read(addr, width)
    }

    pub fn write_sram(&mut self, addr: u32, width: u8, value: u32) {
        self.save.write(addr, width, value);
    }

    pub fn save_type(&self) -> SaveType {
        self.save.save_type()
    }

    /// Feed one EEPROM serial bit (DMA write burst to 0D000000h).
    pub fn eeprom_write_bit(&mut self, bit: bool) {
        self.save.eeprom_write_bit(bit);
    }

    /// Pop one EEPROM response bit (DMA read from 0D000000h).
    pub fn eeprom_read_bit(&mut self) -> bool {
        self.save.eeprom_read_bit()
    }

    /// Peek the EEPROM response level without consuming (CPU load).
    pub fn eeprom_peek_bit(&self) -> bool {
        self.save.eeprom_peek_bit()
    }

    /// End of a DMA burst touching the backup chip.
    pub fn eeprom_end_burst(&mut self) {
        self.save.eeprom_end_burst();
    }

    pub fn has_battery(&self) -> bool {
        self.save.has_battery()
    }

    pub fn ram_data(&self) -> Option<&[u8]> {
        self.save.ram_data()
    }

    pub fn ram_restore(&mut self, data: &[u8]) {
        self.save.ram_restore(data);
    }

    pub(crate) fn export_state(&self) -> CartridgeState {
        CartridgeState {
            save_kind: SaveTypeSer::from(self.save.save_type()),
            save_blob: serde_bytes::ByteBuf::from(self.save.serialize_state()),
            gpio: self.gpio,
        }
    }

    pub(crate) fn import_state(&mut self, state: &CartridgeState) -> Result<(), String> {
        state.validate_against(self.save.save_type())?;
        state.gpio.validate()?;
        self.save
            .deserialize_state(&state.save_blob)
            .map_err(|e| format!("cartridge backend rejected state: {e}"))?;
        self.gpio = state.gpio;
        Ok(())
    }
}

/// GamePak open bus beyond the cartridge (and with no cartridge):
/// the incrementing (Address/2 AND FFFFh) pattern (GBATEK "Unpredictable
/// Things"). 16-bit units are addressed by (addr&!1)>>1; bytes select
/// their lane of that unit; words combine two consecutive units.
fn oob_pattern(addr: u32, width: u8) -> u32 {
    let half = |a: u32| (a >> 1) & 0xFFFF;
    match width {
        4 => {
            let base = addr & !3;
            half(base) | (half(base.wrapping_add(2)) << 16)
        }
        2 => half(addr & !1),
        _ => {
            let h = half(addr & !1);
            (h >> ((addr & 1) * 8)) & 0xFF
        }
    }
}
#[cfg(test)]
mod tests {
    use super::header::finalize_test_gba_rom;
    use super::*;

    fn make_rom_with_save(marker: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x1000];
        finalize_test_gba_rom(&mut rom);
        let off = 0x200;
        rom[off..off + marker.len()].copy_from_slice(marker);
        rom
    }

    #[test]
    fn rom_3mirrors_same() {
        let mut rom = vec![0u8; 0x8000];
        finalize_test_gba_rom(&mut rom);
        for (i, byte) in rom.iter_mut().enumerate() {
            *byte = (i & 0xFF) as u8;
        }
        let cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.read_rom(0x08000000, 4), cart.read_rom(0x0A000000, 4));
        assert_eq!(cart.read_rom(0x08000000, 4), cart.read_rom(0x0C000000, 4));
    }

    #[test]
    fn non_power_of_two_rom_3mirrors_same() {
        let mut rom = vec![0u8; 0x1001];
        finalize_test_gba_rom(&mut rom);
        rom[0x1000] = 0xA5;
        let cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.read_rom(0x08001000, 1), 0xA5);
        assert_eq!(cart.read_rom(0x0A001000, 1), 0xA5);
        assert_eq!(cart.read_rom(0x0C001000, 1), 0xA5);
    }

    #[test]
    fn sram_rw() {
        let rom = make_rom_with_save(b"SRAM_V100");
        let mut cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.save_type(), SaveType::Sram);
        cart.write_sram(0x0E000000, 1, 0x42);
        assert_eq!(cart.read_sram(0x0E000000, 1), 0x42);
    }

    #[test]
    fn detect_flash128_priority() {
        let rom = make_rom_with_save(b"FLASH1M_V102");
        let cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.save_type(), SaveType::Flash128);
    }

    #[test]
    fn rom_oob_returns_address_over_two_pattern() {
        // mgba-suite "ROM out-of-bounds load" (suite.gba is 512KiB;
        // 0x092468AC sits ~19MiB past the end).
        let mut rom = vec![0u8; 0x80000];
        finalize_test_gba_rom(&mut rom);
        rom[0x100] = 0xA5;
        let cart = Cartridge::new(rom).unwrap();
        let base = 0x092468AC;
        assert_eq!(cart.read_rom(base, 1), 0x56);
        assert_eq!(cart.read_rom(base, 2), 0x3456);
        assert_eq!(cart.read_rom(base, 4), 0x34573456);
        // Odd byte selects the high lane of the same 16-bit unit.
        assert_eq!(cart.read_rom(base + 1, 1), 0x34);
        assert_eq!(cart.read_rom(base + 1, 2), 0x3456);
        // In-range reads still return ROM contents.
        assert_eq!(cart.read_rom(0x08000100, 1), 0xA5);
    }

    #[test]
    fn flash_cmd_aa_55_90() {
        let rom = make_rom_with_save(b"FLASH_V130");
        let mut cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.save_type(), SaveType::Flash64);
        cart.write_sram(0x0E005555, 1, 0xAA);
        cart.write_sram(0x0E002AAA, 1, 0x55);
        cart.write_sram(0x0E005555, 1, 0x90);
        // ID mode should be active
        let manuf = cart.read_sram(0x0E000000, 1);
        assert_eq!(manuf, 0x32);
    }

    #[test]
    fn cartridge_state_round_trips_sram_and_gpio() {
        let rom = make_rom_with_save(b"SRAM_V100");
        let mut cart = Cartridge::new(rom).unwrap();
        cart.write_sram(0x0E000123, 1, 0x5A);
        // Attach GPIO with latched data/direction.
        cart.gpio.write(0x080000C8, 2, 1);
        cart.gpio.write(0x080000C6, 2, 0xF);
        cart.gpio.write(0x080000C4, 2, 0xA);
        assert!(cart.gpio.is_attached());

        let state = cart.export_state();
        state
            .validate_against(cart.save.save_type())
            .expect("kind matches");
        let bytes = rmp_serde::to_vec_named(&state).unwrap();
        let decoded: CartridgeState = rmp_serde::from_slice(&bytes).unwrap();

        let rom2 = make_rom_with_save(b"SRAM_V100");
        let mut restored = Cartridge::new(rom2).unwrap();
        restored.import_state(&decoded).unwrap();
        assert_eq!(restored.read_sram(0x0E000123, 1), 0x5A);
        assert!(restored.gpio.is_attached());
        assert_eq!(restored.gpio.read(0x080000C4, 2), Some(0xA));
        let again = rmp_serde::to_vec_named(&restored.export_state()).unwrap();
        assert_eq!(bytes, again);

        // A kind mismatch refuses without touching the backend.
        let mut wrong = decoded.clone();
        wrong.save_kind = crate::rom_identity::SaveTypeSer::Flash64;
        assert!(restored.import_state(&wrong).is_err());
        assert_eq!(restored.read_sram(0x0E000123, 1), 0x5A);
    }

    #[test]
    fn cartridge_state_round_trips_flash_command_state() {
        let rom = make_rom_with_save(b"FLASH_V130");
        let mut cart = Cartridge::new(rom).unwrap();
        cart.write_sram(0x0E005555, 1, 0xAA);
        cart.write_sram(0x0E002AAA, 1, 0x55);
        // Mid-command (Unlock2): the state machine must survive.
        let state = cart.export_state();
        let bytes = rmp_serde::to_vec_named(&state).unwrap();
        let decoded: CartridgeState = rmp_serde::from_slice(&bytes).unwrap();
        let rom2 = make_rom_with_save(b"FLASH_V130");
        let mut restored = Cartridge::new(rom2).unwrap();
        restored.import_state(&decoded).unwrap();
        // Completing the command after restore still enters ID mode.
        restored.write_sram(0x0E005555, 1, 0x90);
        assert_eq!(restored.read_sram(0x0E000000, 1), 0x32);
    }
}
