use super::{Mbc, MbcKind};

const BANK_SIZE: usize = 0x8000;

#[derive(Debug, Clone)]
pub struct M161 {
    rom: Vec<u8>,
    bank: u8,
    locked: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct M161MachineState {
    schema_version: u32,
    bank: u8,
    locked: bool,
}

impl M161 {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            bank: 0,
            locked: false,
        }
    }

    fn read(&self, addr: u16) -> u8 {
        usize::from(self.bank)
            .checked_mul(BANK_SIZE)
            .and_then(|start| start.checked_add(usize::from(addr)))
            .and_then(|index| self.rom.get(index))
            .copied()
            .unwrap_or(0xFF)
    }
}

impl Mbc for M161 {
    fn kind(&self) -> MbcKind {
        MbcKind::M161
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.read(addr)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        self.read(addr)
    }

    fn write_rom(&mut self, _addr: u16, value: u8) {
        if !self.locked {
            self.bank = value & 0x07;
            self.locked = true;
        }
    }

    fn reset_runtime(&mut self) {
        self.bank = 0;
        self.locked = false;
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&M161MachineState {
            schema_version: 1,
            bank: self.bank,
            locked: self.locked,
        })
        .expect("M161 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: M161MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != 1 {
            return Err(format!(
                "unsupported M161 machine state version: {}",
                state.schema_version
            ));
        }
        if state.bank > 0x07 {
            return Err("invalid M161 machine state bank".into());
        }
        self.bank = state.bank;
        self.locked = state.locked;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 4 * BANK_SIZE];
        for (bank, bytes) in rom.as_chunks_mut::<BANK_SIZE>().0.iter_mut().enumerate() {
            bytes.fill(bank as u8);
        }
        rom
    }

    #[test]
    fn first_write_switches_both_windows_and_locks() {
        let mut mapper = M161::new(rom());
        mapper.write_rom(0x7FFF, 0xFA);
        assert_eq!(mapper.read_rom0(0), 2);
        assert_eq!(mapper.read_rom_n(0x4000), 2);
        mapper.write_rom(0, 1);
        assert_eq!(mapper.read_rom0(0), 2);
    }

    #[test]
    fn writing_zero_locks_until_reset() {
        let mut mapper = M161::new(rom());
        mapper.write_rom(0, 0);
        mapper.write_rom(0, 3);
        assert_eq!(mapper.read_rom0(0), 0);
        mapper.reset_runtime();
        mapper.write_rom(0, 3);
        assert_eq!(mapper.read_rom0(0), 3);
    }

    #[test]
    fn state_round_trip_preserves_latch() {
        let mut source = M161::new(rom());
        source.write_rom(0, 2);
        let state = source.serialize_state();
        let mut restored = M161::new(rom());
        restored.deserialize_state(&state).unwrap();
        restored.write_rom(0, 1);
        assert_eq!(restored.read_rom0(0), 2);
    }

    #[test]
    fn invalid_state_is_transactional_and_mapper_has_no_save() {
        let mut mapper = M161::new(rom());
        mapper.write_rom(0, 2);
        let before = mapper.serialize_state();
        let mut state: M161MachineState = rmp_serde::from_slice(&before).unwrap();
        state.bank = 8;
        assert!(
            mapper
                .deserialize_state(&rmp_serde::to_vec_named(&state).unwrap())
                .is_err()
        );
        assert_eq!(mapper.serialize_state(), before);
        assert!(
            mapper
                .export_persistent_state(SystemTime::UNIX_EPOCH)
                .unwrap()
                .is_none()
        );
    }
}
