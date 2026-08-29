use super::{Mbc, MbcKind};

const BANK_SIZE: usize = 0x8000;

#[derive(Debug, Clone)]
pub struct WisdomTree {
    rom: Vec<u8>,
    bank: u8,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WisdomTreeMachineState {
    schema_version: u32,
    bank: u8,
}

impl WisdomTree {
    pub fn new(rom: Vec<u8>) -> Self {
        Self { rom, bank: 0 }
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

impl Mbc for WisdomTree {
    fn kind(&self) -> MbcKind {
        MbcKind::WisdomTree
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.read(addr)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        self.read(addr)
    }

    fn write_rom(&mut self, addr: u16, _value: u8) {
        if addr <= 0x3FFF {
            self.bank = (addr & 0x003F) as u8;
        }
    }

    fn reset_runtime(&mut self) {
        self.bank = 0;
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&WisdomTreeMachineState {
            schema_version: 1,
            bank: self.bank,
        })
        .expect("Wisdom Tree machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: WisdomTreeMachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != 1 {
            return Err(format!(
                "unsupported Wisdom Tree machine state version: {}",
                state.schema_version
            ));
        }
        if state.bank > 0x3F {
            return Err("invalid Wisdom Tree machine state bank".into());
        }
        self.bank = state.bank;
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
    fn write_address_selects_both_windows_repeatedly() {
        let mut mapper = WisdomTree::new(rom());
        mapper.write_rom(0x0002, 0xFF);
        assert_eq!(mapper.read_rom0(0), 2);
        assert_eq!(mapper.read_rom_n(0x4000), 2);
        mapper.write_rom(0x1043, 0);
        assert_eq!(mapper.read_rom0(0), 3);
    }

    #[test]
    fn upper_window_writes_are_ignored() {
        let mut mapper = WisdomTree::new(rom());
        mapper.write_rom(0x0001, 0);
        mapper.write_rom(0x4002, 0);
        assert_eq!(mapper.read_rom0(0), 1);
    }

    #[test]
    fn state_round_trip_preserves_bank() {
        let mut source = WisdomTree::new(rom());
        source.write_rom(3, 0);
        let state = source.serialize_state();
        let mut restored = WisdomTree::new(rom());
        restored.deserialize_state(&state).unwrap();
        assert_eq!(restored.read_rom0(0), 3);
    }

    #[test]
    fn invalid_state_is_transactional_and_mapper_has_no_save() {
        let mut mapper = WisdomTree::new(rom());
        mapper.write_rom(2, 0);
        let before = mapper.serialize_state();
        let mut state: WisdomTreeMachineState = rmp_serde::from_slice(&before).unwrap();
        state.bank = 0x40;
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
