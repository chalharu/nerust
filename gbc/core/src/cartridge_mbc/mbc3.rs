use std::time::SystemTime;

use super::{Mbc, MbcKind, decode_persistent_state, encode_persistent_state, rtc::Mbc3Rtc};

const MACHINE_STATE_SCHEMA_VERSION: u32 = 1;

/// MBC3 with optional cartridge RAM and real-time clock.
#[derive(Debug, Clone)]
pub struct Mbc3 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_rtc_enabled: bool,
    rom_bank: u8,
    ram_rtc_select: u8,
    battery: bool,
    rtc: Option<Mbc3Rtc>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc3MachineState {
    schema_version: u32,
    ram_rtc_enabled: bool,
    rom_bank: u8,
    ram_rtc_select: u8,
    #[serde(with = "serde_bytes")]
    ram: Vec<u8>,
    rtc: Option<Mbc3Rtc>,
}

impl Mbc3 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>, battery: bool, has_rtc: bool) -> Self {
        Self {
            rom,
            ram,
            ram_rtc_enabled: false,
            rom_bank: 1,
            ram_rtc_select: 0,
            battery,
            rtc: has_rtc.then(Mbc3Rtc::new),
        }
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        let selector_count = if self.ram.len() > 0x8000 { 8 } else { 4 };
        if self.ram.is_empty() || usize::from(self.ram_rtc_select) >= selector_count {
            return None;
        }
        Some(
            (usize::from(self.ram_rtc_select) * 0x2000 + usize::from(addr - 0xA000))
                % self.ram.len(),
        )
    }

    fn rom_bank_mask(&self) -> u8 {
        if self.rom.len() > 0x20_0000 {
            0xFF
        } else {
            0x7F
        }
    }
}

impl Mbc for Mbc3 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mbc3
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom.get(usize::from(addr)).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let bank_count = self.rom.len() / 0x4000;
        if bank_count == 0 {
            return 0xFF;
        }
        let bank = usize::from(self.rom_bank) % bank_count;
        let offset = bank * 0x4000 + usize::from(addr - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_rtc_enabled = value & 0x0F == 0x0A,
            0x2000..=0x3FFF => self.rom_bank = (value & self.rom_bank_mask()).max(1),
            0x4000..=0x5FFF => self.ram_rtc_select = value,
            0x6000..=0x7FFF => {
                if let Some(rtc) = &mut self.rtc {
                    rtc.write_latch(value);
                }
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_rtc_enabled {
            return 0xFF;
        }
        if let Some(offset) = self.ram_offset(addr) {
            return self.ram[offset];
        }
        self.rtc
            .as_ref()
            .filter(|_| matches!(self.ram_rtc_select, 0x08..=0x0C))
            .map_or(0xFF, |rtc| rtc.read_latched(self.ram_rtc_select))
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_rtc_enabled {
            return;
        }
        if let Some(offset) = self.ram_offset(addr) {
            self.ram[offset] = value;
            return;
        }
        if matches!(self.ram_rtc_select, 0x08..=0x0C)
            && let Some(rtc) = &mut self.rtc
        {
            rtc.write_live(self.ram_rtc_select, value);
        }
    }

    fn has_battery(&self) -> bool {
        self.battery
    }

    fn ram_data(&self) -> Option<&[u8]> {
        (!self.ram.is_empty()).then_some(self.ram.as_slice())
    }

    fn ram_restore(&mut self, data: &[u8]) {
        // Legacy raw-RAM API: partial restores are allowed, oversized input
        // is ignored. New callers should use import_persistent_state(), which
        // validates the exact RAM length and RTC capability.
        if data.len() <= self.ram.len() {
            self.ram[..data.len()].copy_from_slice(data);
        }
    }

    fn has_rtc(&self) -> bool {
        self.rtc.is_some()
    }

    fn step_clock(&mut self) {
        if let Some(rtc) = &mut self.rtc {
            rtc.step_clock();
        }
    }

    fn sync_rtc(&mut self, now: SystemTime) {
        if let Some(rtc) = &mut self.rtc {
            rtc.sync(now);
        }
    }

    fn sync_rtc_from(&mut self, saved_at: SystemTime, now: SystemTime) {
        if let Some(rtc) = &mut self.rtc {
            rtc.sync_from(saved_at, now);
        }
    }

    fn reset_runtime(&mut self) {
        self.ram_rtc_enabled = false;
        self.rom_bank = 1;
        self.ram_rtc_select = 0;
        if let Some(rtc) = &mut self.rtc {
            rtc.reset_runtime();
        }
    }

    fn export_persistent_state(&self, now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        if !self.battery {
            return Ok(None);
        }
        let rtc = self
            .rtc
            .as_ref()
            .map(|rtc| rtc.encode_persistent(now))
            .transpose()?;
        encode_persistent_state(self.kind(), &self.ram, rtc).map(Some)
    }

    fn import_persistent_state(&mut self, data: &[u8]) -> Result<(), String> {
        if !self.battery {
            return Err("cartridge has no battery-backed persistent state".into());
        }
        let state = decode_persistent_state(data, self.kind())?;
        if state.ram.len() != self.ram.len() {
            return Err(format!(
                "persistent RAM length mismatch: expected {}, got {}",
                self.ram.len(),
                state.ram.len()
            ));
        }
        let rtc = match (&self.rtc, state.rtc) {
            (Some(_), Some(data)) => Some(Mbc3Rtc::decode_persistent(&data)?),
            (None, None) => None,
            _ => return Err("persistent RTC capability mismatch".into()),
        };

        self.ram = state.ram;
        self.rtc = rtc;
        Ok(())
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&Mbc3MachineState {
            schema_version: MACHINE_STATE_SCHEMA_VERSION,
            ram_rtc_enabled: self.ram_rtc_enabled,
            rom_bank: self.rom_bank,
            ram_rtc_select: self.ram_rtc_select,
            ram: self.ram.clone(),
            rtc: self.rtc.clone(),
        })
        .expect("MBC3 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc3MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != MACHINE_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported MBC3 machine state version: {}",
                state.schema_version
            ));
        }
        if state.ram.len() != self.ram.len() {
            return Err(format!(
                "MBC3 machine RAM length mismatch: expected {}, got {}",
                self.ram.len(),
                state.ram.len()
            ));
        }
        if state.rtc.is_some() != self.rtc.is_some() {
            return Err("MBC3 machine RTC capability mismatch".into());
        }
        if let Some(rtc) = &state.rtc {
            rtc.validate()?;
        }

        self.ram_rtc_enabled = state.ram_rtc_enabled;
        self.rom_bank = (state.rom_bank & 0x7F).max(1);
        self.ram_rtc_select = state.ram_rtc_select;
        self.ram = state.ram;
        self.rtc = state.rtc;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn enable(mbc: &mut Mbc3) {
        mbc.write_rom(0x0000, 0x0A);
    }

    fn latch(mbc: &mut Mbc3) {
        mbc.write_rom(0x6000, 0);
        mbc.write_rom(0x6000, 1);
    }

    #[test]
    fn bank_zero_is_remapped_to_one() {
        let mut rom = vec![0u8; 0x10000];
        rom[0x4000] = 0x11;
        let mut mbc = Mbc3::new(rom, vec![], false, false);

        mbc.write_rom(0x2000, 0);

        assert_eq!(mbc.read_rom_n(0x4000), 0x11);
    }

    #[test]
    fn ram_banks_are_selected_and_alias_small_ram() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![0; 0x2000], false, false);
        enable(&mut mbc);
        mbc.write_rom(0x4000, 3);
        mbc.write_ram(0xA000, 0x5A);
        mbc.write_rom(0x4000, 0);

        assert_eq!(mbc.read_ram(0xA000), 0x5A);
    }

    #[test]
    fn ram_and_rtc_access_require_enable() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![0; 0x2000], true, true);

        mbc.write_ram(0xA000, 0x5A);
        assert_eq!(mbc.read_ram(0xA000), 0xFF);

        enable(&mut mbc);
        mbc.write_ram(0xA000, 0x5A);
        assert_eq!(mbc.read_ram(0xA000), 0x5A);
    }

    #[test]
    fn rtc_registers_are_selected_and_latched() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![], true, true);
        enable(&mut mbc);
        mbc.write_rom(0x4000, 0x08);
        mbc.write_ram(0xA000, 42);
        latch(&mut mbc);

        assert_eq!(mbc.read_ram(0xA000), 42);
    }

    #[test]
    fn rtc_selector_is_open_bus_without_timer() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![], false, false);
        enable(&mut mbc);
        mbc.write_rom(0x4000, 0x08);
        mbc.write_ram(0xA000, 42);

        assert_eq!(mbc.read_ram(0xA000), 0xFF);
    }

    #[test]
    fn invalid_selector_is_open_bus() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![0; 0x8000], false, true);
        enable(&mut mbc);
        mbc.write_rom(0x4000, 0x04);

        assert_eq!(mbc.read_ram(0xA000), 0xFF);
    }

    #[test]
    fn mbc30_selects_all_eight_ram_banks() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![0; 0x10000], true, true);
        mbc.write_rom(0x0000, 0x0A);

        for bank in 0..8 {
            mbc.write_rom(0x4000, bank);
            mbc.write_ram(0xA000, 0x80 | bank);
        }
        for bank in 0..8 {
            mbc.write_rom(0x4000, bank);
            assert_eq!(mbc.read_ram(0xA000), 0x80 | bank);
        }
    }

    #[test]
    fn mbc30_selects_all_256_rom_banks() {
        let mut rom = vec![0; 0x400000];
        rom[0x80 * 0x4000] = 0x80;
        rom[0xFF * 0x4000] = 0xFF;
        let mut mbc = Mbc3::new(rom, vec![], false, false);

        mbc.write_rom(0x2000, 0x80);
        assert_eq!(mbc.read_rom_n(0x4000), 0x80);
        mbc.write_rom(0x2000, 0xFF);
        assert_eq!(mbc.read_rom_n(0x4000), 0xFF);
    }

    #[test]
    fn regular_mbc3_masks_rom_bank_to_seven_bits() {
        let mut rom = vec![0; 0x200000];
        rom[0x7F * 0x4000] = 0x7F;
        let mut mbc = Mbc3::new(rom, vec![], false, false);

        mbc.write_rom(0x2000, 0xFF);
        assert_eq!(mbc.read_rom_n(0x4000), 0x7F);
    }

    #[test]
    fn regular_mbc3_rejects_ram_banks_above_capacity() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![0; 0x8000], true, true);
        mbc.write_rom(0x0000, 0x0A);
        mbc.write_rom(0x4000, 4);

        assert_eq!(mbc.read_ram(0xA000), 0xFF);
    }

    #[test]
    fn oversized_legacy_ram_restore_leaves_ram_unchanged() {
        let mut mbc = Mbc3::new(vec![0; 0x8000], vec![0x11; 4], true, false);

        mbc.ram_restore(&[0x22; 5]);

        assert_eq!(mbc.ram, vec![0x11; 4]);
    }

    #[test]
    fn persistent_state_restores_ram_and_offline_rtc_elapsed_time() {
        let mut source = Mbc3::new(vec![0; 0x8000], vec![0; 0x2000], true, true);
        enable(&mut source);
        source.write_ram(0xA000, 0xA5);
        let saved = source
            .export_persistent_state(UNIX_EPOCH + Duration::from_secs(100))
            .expect("export")
            .expect("persistent state");
        let mut restored = Mbc3::new(vec![0; 0x8000], vec![0; 0x2000], true, true);

        restored.import_persistent_state(&saved).expect("import");
        restored.sync_rtc(UNIX_EPOCH + Duration::from_secs(105));
        enable(&mut restored);
        restored.write_rom(0x4000, 0x08);
        latch(&mut restored);

        assert_eq!(restored.ram[0], 0xA5);
        assert_eq!(restored.read_ram(0xA000), 5);
    }

    #[test]
    fn machine_state_round_trip_restores_runtime_registers() {
        let mut source = Mbc3::new(vec![0; 0x10000], vec![0; 0x2000], true, true);
        enable(&mut source);
        source.write_rom(0x2000, 2);
        source.write_rom(0x4000, 0x08);
        source.write_ram(0xA000, 33);
        latch(&mut source);
        let state = source.serialize_state();
        let mut restored = Mbc3::new(vec![0; 0x10000], vec![0; 0x2000], true, true);

        restored.deserialize_state(&state).expect("restore");

        assert!(restored.ram_rtc_enabled);
        assert_eq!(restored.rom_bank, 2);
        assert_eq!(restored.ram_rtc_select, 0x08);
        assert_eq!(restored.read_ram(0xA000), 33);
    }
}
