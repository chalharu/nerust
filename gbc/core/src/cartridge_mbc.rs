use std::time::SystemTime;

use crate::cartridge_header::CartridgeHeader;

mod mbc3;
mod mbc5;
mod rtc;

pub use mbc3::Mbc3;
pub use mbc5::Mbc5;

const PERSISTENT_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MbcKind {
    RomOnly,
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MbcPersistentState {
    schema_version: u32,
    kind: MbcKind,
    #[serde(with = "serde_bytes")]
    ram: Vec<u8>,
    rtc: Option<Vec<u8>>,
}

fn encode_persistent_state(
    kind: MbcKind,
    ram: &[u8],
    rtc: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(&MbcPersistentState {
        schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
        kind,
        ram: ram.to_vec(),
        rtc,
    })
    .map_err(|error| error.to_string())
}

fn decode_persistent_state(data: &[u8], kind: MbcKind) -> Result<MbcPersistentState, String> {
    let state: MbcPersistentState =
        rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
    if state.schema_version != PERSISTENT_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported MBC persistent state version: {}",
            state.schema_version
        ));
    }
    if state.kind != kind {
        return Err(format!(
            "MBC persistent state kind mismatch: expected {kind:?}, got {:?}",
            state.kind
        ));
    }
    Ok(state)
}

/// Memory Bank Controller trait.
///
/// Handles ROM bank switching, RAM access, and battery-backed save data.
/// Default implementations are no-ops so ROM Only only needs to implement
/// `read_rom0`, `read_rom_n`, `serialize_state`, and `deserialize_state`.
#[allow(unused_variables)]
pub trait Mbc: std::fmt::Debug + Send {
    fn kind(&self) -> MbcKind;
    fn read_rom0(&self, addr: u16) -> u8;
    fn read_rom_n(&self, addr: u16) -> u8;

    fn write_rom(&mut self, addr: u16, value: u8) {}

    fn read_ram(&self, addr: u16) -> u8 {
        0xFF
    }
    fn write_ram(&mut self, addr: u16, value: u8) {}

    fn has_battery(&self) -> bool {
        false
    }
    fn ram_data(&self) -> Option<&[u8]> {
        None
    }
    fn ram_restore(&mut self, data: &[u8]) {}

    fn has_rtc(&self) -> bool {
        false
    }
    fn step_clock(&mut self) {}
    fn sync_rtc(&mut self, now: SystemTime) {}
    fn sync_rtc_from(&mut self, saved_at: SystemTime, now: SystemTime) {}

    fn reset_runtime(&mut self) {}

    fn export_persistent_state(&self, _now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        if !self.has_battery() {
            return Ok(None);
        }
        encode_persistent_state(self.kind(), self.ram_data().unwrap_or_default(), None).map(Some)
    }

    fn import_persistent_state(&mut self, data: &[u8]) -> Result<(), String> {
        if !self.has_battery() {
            return Err("cartridge has no battery-backed persistent state".into());
        }
        let state = decode_persistent_state(data, self.kind())?;
        if state.rtc.is_some() {
            return Err("unexpected RTC data for cartridge".into());
        }
        let expected_len = self.ram_data().map_or(0, <[u8]>::len);
        if state.ram.len() != expected_len {
            return Err(format!(
                "persistent RAM length mismatch: expected {expected_len}, got {}",
                state.ram.len()
            ));
        }
        self.ram_restore(&state.ram);
        Ok(())
    }

    fn serialize_state(&self) -> Vec<u8>;
    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String>;
}

/// ROM Only MBC: no banking, no RAM.
#[derive(Debug, Clone)]
pub struct RomOnly {
    rom: Vec<u8>,
}

impl RomOnly {
    pub fn new(rom: Vec<u8>) -> Self {
        Self { rom }
    }
}

impl Mbc for RomOnly {
    fn kind(&self) -> MbcKind {
        MbcKind::RomOnly
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom[addr as usize]
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        self.rom[addr as usize]
    }

    fn serialize_state(&self) -> Vec<u8> {
        Vec::new()
    }

    fn deserialize_state(&mut self, _data: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// MBC1 (max 2 MiB ROM and/or 32 KiB RAM).
#[derive(Debug, Clone)]
pub struct Mbc1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_enabled: bool,
    rom_bank: u8,
    ram_bank: u8,
    banking_mode: bool,
    battery: bool,
    rom_bank_mask: u8,
    /// 8 Mbit MBC1 multicart: the ROM is laid out as several games, and the
    /// bank registers select game + bank with a different bit layout
    /// (bank1 low 4 bits select the bank, bank2 selects the game via << 4).
    multicart: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc1MachineState {
    schema_version: u32,
    ram_enabled: bool,
    rom_bank: u8,
    ram_bank: u8,
    banking_mode: bool,
    ram: Vec<u8>,
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>, battery: bool) -> Self {
        Self::with_multicart(rom, ram, battery, false)
    }

    pub fn new_multicart(rom: Vec<u8>, ram: Vec<u8>, battery: bool) -> Self {
        Self::with_multicart(rom, ram, battery, true)
    }

    fn with_multicart(rom: Vec<u8>, ram: Vec<u8>, battery: bool, multicart: bool) -> Self {
        let rom_bank_mask = Self::bank_mask(rom.len() / 0x4000);
        Self {
            rom,
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            banking_mode: false,
            battery,
            rom_bank_mask,
            multicart,
        }
    }

    fn bank_mask(bank_count: usize) -> u8 {
        bank_count.saturating_sub(1) as u8
    }

    /// Lower (bank 0 / $0000-$3FFF) and upper (banked / $4000-$7FFF) ROM bank
    /// numbers, following the MBC1 mode and (for 8 Mbit multicarts) the
    /// multicart bank bit layout.
    fn bank_layout(&self) -> (usize, usize) {
        let bank = if self.rom_bank == 0 { 1 } else { self.rom_bank };
        let (upper_bits, lower_bits) = if self.multicart {
            ((self.ram_bank as usize) << 4, (bank as usize) & 0x0F)
        } else {
            ((self.ram_bank as usize) << 5, bank as usize)
        };
        let lower_bank = if self.banking_mode { upper_bits } else { 0 };
        (lower_bank, upper_bits | lower_bits)
    }

    fn rom_bank_effective(&self) -> usize {
        let (_, upper_bank) = self.bank_layout();
        upper_bank & self.rom_bank_mask as usize
    }
}

impl Mbc for Mbc1 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mbc1
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        let (lower_bank, _) = self.bank_layout();
        let offset = (lower_bank & self.rom_bank_mask as usize) * 0x4000 + addr as usize;
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let offset = self.rom_bank_effective() * 0x4000 + (addr as usize - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = (value & 0x0F) == 0x0A;
            }
            0x2000..=0x3FFF => {
                self.rom_bank = value & 0x1F;
            }
            0x4000..=0x5FFF => {
                self.ram_bank = value & 0x03;
            }
            0x6000..=0x7FFF => {
                self.banking_mode = (value & 0x01) != 0;
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xFF;
        }
        let bank = if self.banking_mode {
            self.ram_bank as usize
        } else {
            0
        };
        // Mask the banked address into the physical RAM size: cartridges with
        // a small RAM chip only wire the low address lines, so RAM bank bits
        // beyond the chip size alias back to the start of the chip.
        let mask = self.ram.len() - 1;
        let offset = ((bank * 0x2000) | (addr as usize - 0xA000)) & mask;
        self.ram[offset]
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let bank = if self.banking_mode {
            self.ram_bank as usize
        } else {
            0
        };
        let mask = self.ram.len() - 1;
        let offset = ((bank * 0x2000) | (addr as usize - 0xA000)) & mask;
        self.ram[offset] = value;
    }

    fn has_battery(&self) -> bool {
        self.battery
    }

    fn ram_data(&self) -> Option<&[u8]> {
        if self.ram.is_empty() {
            None
        } else {
            Some(&self.ram)
        }
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() <= self.ram.len() {
            self.ram[..data.len()].copy_from_slice(data);
        }
    }

    fn reset_runtime(&mut self) {
        self.ram_enabled = false;
        self.rom_bank = 1;
        self.ram_bank = 0;
        self.banking_mode = false;
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&Mbc1MachineState {
            schema_version: 1,
            ram_enabled: self.ram_enabled,
            rom_bank: self.rom_bank,
            ram_bank: self.ram_bank,
            banking_mode: self.banking_mode,
            ram: self.ram.clone(),
        })
        .expect("MBC1 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc1MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != 1 {
            return Err(format!(
                "unsupported MBC1 machine state version: {}",
                state.schema_version
            ));
        }
        if state.ram.len() != self.ram.len() {
            return Err("MBC1 machine state RAM length mismatch".into());
        }
        self.ram_enabled = state.ram_enabled;
        self.rom_bank = state.rom_bank;
        self.ram_bank = state.ram_bank & 0x03;
        self.banking_mode = state.banking_mode;
        self.ram = state.ram;
        Ok(())
    }
}

/// MBC2 (up to 256 KiB ROM, built-in 512-nibble RAM).
#[derive(Debug, Clone)]
pub struct Mbc2 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_enabled: bool,
    rom_bank: u8,
    battery: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc2MachineState {
    schema_version: u32,
    ram_enabled: bool,
    rom_bank: u8,
    ram: Vec<u8>,
}

impl Mbc2 {
    pub fn new(rom: Vec<u8>, battery: bool) -> Self {
        Self {
            rom,
            ram: vec![0; 0x200],
            ram_enabled: false,
            rom_bank: 1,
            battery,
        }
    }
}

impl Mbc for Mbc2 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mbc2
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom.get(addr as usize).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let bank = if self.rom_bank == 0 {
            1
        } else {
            self.rom_bank as usize
        };
        let bank_count = self.rom.len() / 0x4000;
        let bank = if bank_count > 0 {
            bank & (bank_count - 1)
        } else {
            0
        };
        let offset = bank * 0x4000 + (addr as usize - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        // MBC2 control registers only live at $0000-$3FFF; writes to
        // $4000-$7FFF (no RAM banking) are ignored.
        if addr >= 0x4000 {
            return;
        }
        if addr & 0x0100 == 0 {
            // RAM enable
            self.ram_enabled = (value & 0x0F) == 0x0A;
        } else {
            // ROM bank select (lower 4 bits)
            self.rom_bank = value & 0x0F;
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xFF;
        }
        let idx = (addr as usize - 0xA000) & 0x1FF;
        self.ram.get(idx).copied().unwrap_or(0xFF) | 0xF0
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let idx = (addr as usize - 0xA000) & 0x1FF;
        if let Some(cell) = self.ram.get_mut(idx) {
            *cell = value & 0x0F;
        }
    }

    fn has_battery(&self) -> bool {
        self.battery
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.ram)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() <= self.ram.len() {
            self.ram[..data.len()].copy_from_slice(data);
        }
    }

    fn reset_runtime(&mut self) {
        self.ram_enabled = false;
        self.rom_bank = 1;
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&Mbc2MachineState {
            schema_version: 1,
            ram_enabled: self.ram_enabled,
            rom_bank: self.rom_bank,
            ram: self.ram.clone(),
        })
        .expect("MBC2 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc2MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != 1 {
            return Err(format!(
                "unsupported MBC2 machine state version: {}",
                state.schema_version
            ));
        }
        if state.ram.len() != self.ram.len() || state.ram.iter().any(|value| value & 0xF0 != 0) {
            return Err("invalid MBC2 machine state RAM".into());
        }
        self.ram_enabled = state.ram_enabled;
        self.rom_bank = state.rom_bank & 0x0F;
        self.ram = state.ram;
        Ok(())
    }
}

/// Factory function to create the appropriate MBC from header + ROM data.
pub fn create_mbc(header: &CartridgeHeader, rom: Vec<u8>, ram: Option<Vec<u8>>) -> Box<dyn Mbc> {
    match header.cartridge_type {
        crate::cartridge_header::CartridgeType::RomOnly => Box::new(RomOnly::new(rom)),
        crate::cartridge_header::CartridgeType::Mbc1
        | crate::cartridge_header::CartridgeType::Mbc1Ram
        | crate::cartridge_header::CartridgeType::Mbc1RamBattery => {
            let ram_size = if header.cartridge_type.has_ram() && header.ram_size.bytes == 0 {
                0x2000
            } else {
                header.ram_size.bytes
            };
            let ram = ram.unwrap_or_else(|| vec![0; ram_size]);
            if header.multicart {
                Box::new(Mbc1::new_multicart(
                    rom,
                    ram,
                    header.cartridge_type.has_battery(),
                ))
            } else {
                Box::new(Mbc1::new(rom, ram, header.cartridge_type.has_battery()))
            }
        }
        crate::cartridge_header::CartridgeType::Mbc5
        | crate::cartridge_header::CartridgeType::Mbc5Ram
        | crate::cartridge_header::CartridgeType::Mbc5RamBattery
        | crate::cartridge_header::CartridgeType::Mbc5Rumble
        | crate::cartridge_header::CartridgeType::Mbc5RumbleRam
        | crate::cartridge_header::CartridgeType::Mbc5RumbleRamBattery => {
            let ram_size = if header.cartridge_type.has_ram() && header.ram_size.bytes == 0 {
                0x2000
            } else {
                header.ram_size.bytes
            };
            let ram = ram.unwrap_or_else(|| vec![0; ram_size]);
            Box::new(Mbc5::new(
                rom,
                ram,
                header.cartridge_type.has_battery(),
                header.cartridge_type.has_rumble(),
            ))
        }
        crate::cartridge_header::CartridgeType::Mbc3TimerBattery
        | crate::cartridge_header::CartridgeType::Mbc3TimerRamBattery
        | crate::cartridge_header::CartridgeType::Mbc3
        | crate::cartridge_header::CartridgeType::Mbc3Ram
        | crate::cartridge_header::CartridgeType::Mbc3RamBattery => {
            let ram = ram.unwrap_or_else(|| vec![0; header.ram_size.bytes]);
            Box::new(Mbc3::new(
                rom,
                ram,
                header.cartridge_type.has_battery(),
                header.cartridge_type.has_rtc(),
            ))
        }
        crate::cartridge_header::CartridgeType::Mbc2
        | crate::cartridge_header::CartridgeType::Mbc2Battery => {
            Box::new(Mbc2::new(rom, header.cartridge_type.has_battery()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_only_reads_all_addresses_from_rom() {
        let rom = vec![0x42u8; 0x8000];
        let mbc = RomOnly::new(rom);
        assert_eq!(mbc.read_rom0(0x0000), 0x42);
        assert_eq!(mbc.read_rom_n(0x4000), 0x42);
    }

    #[test]
    fn rom_only_has_no_battery_by_default() {
        let mbc = RomOnly::new(vec![0; 0x8000]);
        assert!(!mbc.has_battery());
    }

    #[test]
    fn mbc1_default_reads_bank_0_and_1() {
        let mut rom = vec![0u8; 0x20000]; // 8 banks → 128 KiB
        rom[0x0000] = 0xAA;
        rom[0x4000] = 0xBB;
        let mbc = Mbc1::new(rom, vec![0; 0x2000], false);
        assert_eq!(mbc.read_rom0(0x0000), 0xAA);
        assert_eq!(mbc.read_rom_n(0x4000), 0xBB);
    }

    #[test]
    fn mbc1_bank_switch_reads_correct_bank() {
        let mut rom = vec![0u8; 0x20000]; // 8 banks
        rom[0x8000] = 0xCC; // bank 2, offset 0
        let mut mbc = Mbc1::new(rom, vec![0; 0x2000], false);
        mbc.write_rom(0x2000, 0x02); // select bank 2
        assert_eq!(mbc.read_rom_n(0x4000), 0xCC);
    }

    #[test]
    fn mbc1_ram_read_requires_enable() {
        let mut ram = vec![0u8; 0x2000];
        ram[0] = 0x77;
        let mut mbc = Mbc1::new(vec![0; 0x8000], ram, false);
        assert_eq!(mbc.read_ram(0xA000), 0xFF); // disabled
        mbc.write_rom(0x0000, 0x0A); // enable RAM
        assert_eq!(mbc.read_ram(0xA000), 0x77);
    }

    #[test]
    fn mbc1_ram_type_with_zero_size_gets_minimum_ram() {
        let mut rom = vec![0; 0x8000];
        rom[0x0147] = 0x02;
        rom[0x0148] = 0x00;
        rom[0x0149] = 0x00;
        let header = CartridgeHeader::parse(&rom).unwrap();
        let mut mbc = create_mbc(&header, rom, None);

        mbc.write_rom(0x0000, 0x0A);
        mbc.write_ram(0xA000, 0x5A);

        assert_eq!(mbc.read_ram(0xA000), 0x5A);
    }

    #[test]
    fn mbc3_cartridge_types_have_expected_capabilities() {
        let cases = [
            (0x0F, 0x00, true, true, false),
            (0x10, 0x02, true, true, true),
            (0x11, 0x00, false, false, false),
            (0x12, 0x02, false, false, true),
            (0x13, 0x02, false, true, true),
        ];
        for (cartridge_type, ram_size, has_rtc, has_battery, has_ram) in cases {
            let mut rom = vec![0; 0x8000];
            rom[0x0147] = cartridge_type;
            rom[0x0148] = 0x00;
            rom[0x0149] = ram_size;
            let header = CartridgeHeader::parse(&rom).expect("header");
            let mbc = create_mbc(&header, rom, None);

            assert_eq!(mbc.has_rtc(), has_rtc, "type {cartridge_type:#04X}");
            assert_eq!(mbc.has_battery(), has_battery, "type {cartridge_type:#04X}");
            assert_eq!(
                mbc.ram_data().is_some(),
                has_ram,
                "type {cartridge_type:#04X}"
            );
        }
    }

    #[test]
    fn mbc1_bank_0_treated_as_1() {
        let mut rom = vec![0u8; 0x10000]; // 4 banks
        rom[0x0000] = 0xAA;
        rom[0x4000] = 0x11;
        let mut mbc = Mbc1::new(rom, vec![], false);
        mbc.write_rom(0x2000, 0x00); // select bank 0 → treated as 1
        assert_eq!(mbc.read_rom_n(0x4000), 0x11);
    }

    #[test]
    fn mbc1_large_rom_uses_secondary_bank_register() {
        // 2 MiB ROM: 128 banks, uses 2-bit secondary register for bits 5-6
        let mut rom = vec![0u8; 0x200000]; // 128 banks = 2 MiB
        let target_bank = 33; // bank 33 = 32 + 1 → secondary=1, primary=1
        rom[target_bank * 0x4000] = 0xCC;
        let mut mbc = Mbc1::new(rom, vec![], false);
        mbc.write_rom(0x4000, 0x01); // secondary bank = 1
        mbc.write_rom(0x2000, 0x01); // primary bank = 1
        // Effective = (1 << 5) | 1 = 33
        assert_eq!(mbc.read_rom_n(0x4000), 0xCC);
    }

    #[test]
    fn mbc1_mode_1_maps_rom0_to_other_bank() {
        let mut rom = vec![0u8; 0x200000]; // 128 banks (2 MiB)
        let bank32 = 32 * 0x4000;
        rom[bank32] = 0xDD;
        let mut mbc = Mbc1::new(rom, vec![], false);
        mbc.write_rom(0x4000, 0x01); // secondary = 1
        mbc.write_rom(0x6000, 0x01); // mode 1
        assert_eq!(mbc.read_rom0(0x0000), 0xDD); // 0000 reads from bank $20
    }

    #[test]
    fn mbc1_mode_1_allows_ram_banking() {
        let mut ram = vec![0u8; 0x8000]; // 32 KiB (4 banks)
        ram[0] = 0x11;
        ram[0x2000] = 0x22;
        let mut mbc = Mbc1::new(vec![0; 0x8000], ram, false);
        mbc.write_rom(0x0000, 0x0A); // enable
        mbc.write_rom(0x4000, 0x01); // ram_bank = 1
        mbc.write_rom(0x6000, 0x01); // mode 1
        assert_eq!(mbc.read_ram(0xA000), 0x22); // reads bank 1
    }

    #[test]
    fn mbc1_mode_0_locks_ram_to_bank_0() {
        let mut ram = vec![0u8; 0x8000];
        ram[0] = 0x11;
        ram[0x2000] = 0x22;
        let mut mbc = Mbc1::new(vec![0; 0x8000], ram, false);
        mbc.write_rom(0x0000, 0x0A); // enable
        mbc.write_rom(0x4000, 0x01); // ram_bank = 1
        mbc.write_rom(0x6000, 0x00); // mode 0
        assert_eq!(mbc.read_ram(0xA000), 0x11); // locked to bank 0
    }

    #[test]
    fn mbc1_deserialize_state_restores_registers() {
        let mut mbc = Mbc1::new(vec![0; 0x20000], vec![0; 0x2000], true);
        mbc.write_rom(0x0000, 0x0A); // enable
        mbc.write_rom(0x2000, 0x03); // rom_bank = 3
        mbc.write_rom(0x4000, 0x02); // ram_bank = 2
        mbc.write_rom(0x6000, 0x01); // mode 1

        let state = mbc.serialize_state();
        let mut restored = Mbc1::new(vec![0; 0x20000], vec![0; 0x2000], true);
        restored.deserialize_state(&state).expect("deserialize");

        let mut rom_set = vec![0u8; 0x20000];
        rom_set[3 * 0x4000] = 0xFF;
        let mut mbc2 = Mbc1::new(rom_set, vec![0; 0x2000], false);
        mbc2.deserialize_state(&state).expect("deserialize 2");
        assert_eq!(mbc2.read_rom_n(0x4000), 0xFF);

        let mut incompatible = Mbc1::new(vec![0; 0x20000], vec![], false);
        assert!(incompatible.deserialize_state(&state).is_err());
    }

    #[test]
    fn serialize_state_round_trip() {
        let mut mbc = Mbc1::new(vec![0; 0x8000], vec![0x42; 0x2000], false);
        mbc.write_rom(0x2000, 0x05);
        let state = mbc.serialize_state();

        let mut restored = Mbc1::new(vec![0; 0x8000], vec![0; 0x2000], false);
        restored.deserialize_state(&state).expect("ok");
        assert_eq!(state, restored.serialize_state());
    }

    #[test]
    fn bank_mask_edge_cases() {
        assert_eq!(Mbc1::bank_mask(1), 0);
        assert_eq!(Mbc1::bank_mask(2), 1);
        assert_eq!(Mbc1::bank_mask(128), 127);
    }

    #[test]
    fn persistent_state_round_trip_restores_battery_ram() {
        let mut source = Mbc1::new(vec![0; 0x8000], vec![0; 0x2000], true);
        source.write_rom(0x0000, 0x0A);
        source.write_ram(0xA000, 0x5A);
        let state = source
            .export_persistent_state(SystemTime::UNIX_EPOCH)
            .expect("export")
            .expect("battery state");

        let mut restored = Mbc1::new(vec![0; 0x8000], vec![0; 0x2000], true);
        restored.import_persistent_state(&state).expect("import");
        restored.write_rom(0x0000, 0x0A);

        assert_eq!(restored.read_ram(0xA000), 0x5A);
    }

    #[test]
    fn persistent_state_rejects_different_mbc_kind() {
        let source = Mbc1::new(vec![0; 0x8000], vec![0; 0x2000], true);
        let state = source
            .export_persistent_state(SystemTime::UNIX_EPOCH)
            .expect("export")
            .expect("battery state");
        let mut target = Mbc2::new(vec![0; 0x8000], true);

        assert!(target.import_persistent_state(&state).is_err());
    }

    #[test]
    fn cartridge_without_battery_has_no_persistent_state() {
        let mbc = Mbc1::new(vec![0; 0x8000], vec![0; 0x2000], false);

        assert_eq!(
            mbc.export_persistent_state(SystemTime::UNIX_EPOCH)
                .expect("export"),
            None
        );
    }
}
