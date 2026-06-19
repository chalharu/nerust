// Mapper 1

use super::Cartridge;
use crate::cartridge_rom::CartridgeData;
use crate::cartridge_runtime_state::{CartridgeRuntimeState, MAPPER_KIND_SXROM};
use crate::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::persistence_codec::{decode_payload, encode_payload};
use crate::persistence_error::PersistenceError;
use nerust_contract_core::mirror::MirrorMode;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SxRom {
    cartridge_data: CartridgeData,
    state: MapperState,
    control: u8,    // 0x8000 - 0x9FFF
    chr_bank_0: u8, // 0xA000 - 0xBFFF
    chr_bank_1: u8, // 0xC000 - 0xDFFF
    prg_bank: u8,   // 0xE000 - 0xFFFF
    shift_register: u8,
    last_chr_bank: bool, // false: bank0, true: bank1
    cycle: u64,
    prev_cycle: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SxRomRuntimeState {
    control: u8,
    chr_bank_0: u8,
    chr_bank_1: u8,
    prg_bank: u8,
    shift_register: u8,
    last_chr_bank: bool,
    cycle: u64,
    prev_cycle: u64,
}

#[typetag::serde]
impl Cartridge for SxRom {
    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        Ok(CartridgeRuntimeState {
            mapper_state: self.state.clone(),
            extra_kind: MAPPER_KIND_SXROM.into(),
            extra_body: encode_payload(&SxRomRuntimeState {
                control: self.control,
                chr_bank_0: self.chr_bank_0,
                chr_bank_1: self.chr_bank_1,
                prg_bank: self.prg_bank,
                shift_register: self.shift_register,
                last_chr_bank: self.last_chr_bank,
                cycle: self.cycle,
                prev_cycle: self.prev_cycle,
            })?,
        })
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        if state.extra_kind != MAPPER_KIND_SXROM {
            return Err(PersistenceError::Validation(
                "unexpected SXROM runtime kind".into(),
            ));
        }
        self.state
            .validate_for_import(
                &state.mapper_state,
                self.data_ref().prog_rom_len(),
                self.data_ref().char_rom_len(),
            )
            .map_err(PersistenceError::Validation)?;
        let runtime: SxRomRuntimeState = decode_payload(&state.extra_body)?;
        self.state = state.mapper_state;
        self.control = runtime.control;
        self.chr_bank_0 = runtime.chr_bank_0;
        self.chr_bank_1 = runtime.chr_bank_1;
        self.prg_bank = runtime.prg_bank;
        self.shift_register = runtime.shift_register;
        self.last_chr_bank = runtime.last_chr_bank;
        self.cycle = runtime.cycle;
        self.prev_cycle = runtime.prev_cycle;
        Ok(())
    }
}

impl SxRom {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            control: 0x0C,
            prg_bank: 0,
            chr_bank_0: 0,
            chr_bank_1: 0,
            shift_register: 0x10,
            cycle: 0,
            prev_cycle: 0,
            last_chr_bank: false,
        }
    }

    fn write_register_inner(&mut self, address: usize, value: u8) {
        match address {
            0..=0x1FFF => self.write_control(value),
            0x2000..=0x3FFF => self.write_char_bank_0(value),
            0x4000..=0x5FFF => self.write_char_bank_1(value),
            0x6000..=0x7FFF => self.write_prog_bank(value),
            _ => {}
        }
    }

    fn write_control(&mut self, value: u8) {
        self.control = value;

        let mirror_mode = match value & 3 {
            0 => MirrorMode::Single0,
            1 => MirrorMode::Single1,
            2 => MirrorMode::Vertical,
            3 => MirrorMode::Horizontal,
            _ => unreachable!(),
        };
        self.set_mirror_mode(mirror_mode);
        self.update_offsets();
    }

    fn write_char_bank_0(&mut self, value: u8) {
        self.chr_bank_0 = value;
        self.update_offsets();
    }

    fn write_char_bank_1(&mut self, value: u8) {
        self.chr_bank_1 = value;
        self.update_offsets();
    }

    fn write_prog_bank(&mut self, value: u8) {
        self.prg_bank = value;
        self.update_offsets();
    }

    fn active_extra_register(&self) -> u8 {
        if self.last_chr_bank && (self.control & 0x10) == 0x10 {
            self.chr_bank_1
        } else {
            self.chr_bank_0
        }
    }

    fn uses_chr_bank_ram_protect(&self) -> bool {
        self.data_ref().char_rom_len() == 0 && self.data_ref().prog_rom_len() <= 0x40000
    }

    fn program_ram_enabled(&self) -> bool {
        !self.mapper_state_ref().sram.is_empty()
            && (self.prg_bank & 0x10) == 0
            && !(self.uses_chr_bank_ram_protect() && (self.active_extra_register() & 0x10) != 0)
    }

    fn update_offsets(&mut self) {
        let extra_reg = self.active_extra_register();

        if (self.prg_bank & 0x10) != 0x10 {
            if self.data_ref().pram_length() + self.data_ref().save_pram_length() > 0x4000 {
                // SXROM ( save 32kb )
                self.change_ram_page(0, usize::from((extra_reg >> 2) & 0x03));
            } else if self.data_ref().pram_length() + self.data_ref().save_pram_length() > 0x2000 {
                if self.data_ref().save_pram_length() == 0x2000
                    && self.data_ref().pram_length() == 0x2000
                {
                    // SOROM ( save 16kb + ram 16kb )
                    self.change_ram_page(
                        0,
                        if (extra_reg >> 3) & 0x01 != 0 {
                            0
                        } else {
                            self.data_ref().save_pram_length() / self.ram_page_len()
                        },
                    );
                } else {
                    // unknown
                    self.change_ram_page(0, usize::from((extra_reg >> 2) & 0x01));
                }
            } else {
                // ram 8kb or nothing
                self.change_ram_page(0, 0);
            }
        }
        if self.data_ref().sub_mapper_type() == 5 {
            self.change_program_page(0, 0);
            self.change_program_page(1, 1);
        } else {
            let prog_bank_sel = if self.data_ref().prog_rom_len() == 0x80000 {
                // 512KB Rom
                extra_reg & 0x10
            } else {
                0
            };
            match (self.control >> 2) & 3 {
                0 | 1 => {
                    // 32k
                    let bank = usize::from((self.prg_bank & 0x0E) | prog_bank_sel);
                    self.change_program_page(0, bank);
                    self.change_program_page(1, bank + 1);
                }
                3 => {
                    // 16k
                    self.change_program_page(
                        0,
                        usize::from((self.prg_bank & 0x0F) | prog_bank_sel),
                    );
                    self.change_program_page(1, usize::from(0x0F | prog_bank_sel));
                }
                _ => {
                    self.change_program_page(0, usize::from(prog_bank_sel));
                    self.change_program_page(
                        1,
                        usize::from((self.prg_bank & 0x0F) | prog_bank_sel),
                    );
                }
            }
        }

        if (self.control & 0x10) == 0x00 {
            // 8k
            self.change_character_page(0, usize::from(self.chr_bank_0 & 0x1E));
            self.change_character_page(1, usize::from((self.chr_bank_0 & 0x1E) + 1));
        } else {
            // 4k
            self.change_character_page(0, usize::from(self.chr_bank_0));
            self.change_character_page(1, usize::from(self.chr_bank_1));
        }
    }
}

impl CartridgeDataDao for SxRom {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for SxRom {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for SxRom {
    fn program_page_len(&self) -> usize {
        0x4000
    }
    fn character_page_len(&self) -> usize {
        0x1000
    }

    fn initialize(&mut self) {
        // MMC1A, MMC1BであればWRAMを有効にする必要がある。

        self.write_control(0x0C);
    }

    fn name(&self) -> &str {
        "MMC1 SXROM (Mapper1)"
    }

    fn bus_conflicts(&self) -> bool {
        self.data_ref().sub_mapper_type() == 2
    }

    fn write_register(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        if self.cycle.wrapping_sub(self.prev_cycle) >= 2 {
            if value & 0x80 == 0x80 {
                self.shift_register = 0x10;
                let control = self.control | 0x0C;
                self.write_control(control);
            } else {
                let complete = self.shift_register & 1 == 1;
                let shift_register = (self.shift_register >> 1) | ((value & 1) << 4);
                self.shift_register = if complete {
                    self.write_register_inner(address & 0x7FFF, shift_register);
                    0x10
                } else {
                    shift_register
                };
            }
            self.prev_cycle = self.cycle;
        }
    }

    fn step(&mut self, _interrupt: &mut Interrupt) {
        self.cycle += 1;
    }

    fn step_cpu_cycles(&mut self, cycles: u64, _interrupt: &mut Interrupt) {
        self.cycle = self.cycle.wrapping_add(cycles);
    }

    fn cpu_read_has_side_effect(&self, _address: usize) -> bool {
        false
    }

    fn allow_instruction_fast_path(&self) -> bool {
        true
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        if self.program_ram_enabled() {
            self.ram_address(index)
                .map(|address| self.mapper_state_ref().sram[address])
        } else {
            None
        }
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        if self.program_ram_enabled()
            && let Some(address) = self.ram_address(index)
        {
            self.mapper_state_mut().sram[address] = data;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cartridge;
    use super::SxRom;
    use crate::cartridge_data_parts::CartridgeDataParts;
    use crate::cartridge_rom::CartridgeData;
    use crate::mapper::Mapper;
    use crate::mapper_state::MapperStateDao;
    use nerust_contract_core::mirror::MirrorMode;
    use nerust_contract_core::rom::RomFormat;

    fn new_mapper(prg_rom_len: usize, chr_rom_len: usize, prg_ram_banks_8k: u8) -> SxRom {
        let data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; prg_rom_len],
            char_rom: vec![0; chr_rom_len],
            pram_length: usize::from(prg_ram_banks_8k.max(1)) * 0x2000,
            save_pram_length: 0,
            vram_length: if chr_rom_len == 0 { 0x2000 } else { 0 },
            save_vram_length: 0,
            mapper_type: 1,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid");
        let mut mapper = SxRom::new(data);
        Cartridge::initialize(&mut mapper);
        mapper
    }

    fn write_program_ram(mapper: &mut SxRom, value: u8) {
        <SxRom as Mapper>::write_ram(mapper, 0x0000, value);
    }

    fn read_program_ram(mapper: &SxRom) -> Option<u8> {
        <SxRom as Mapper>::read_ram(mapper, 0x0000)
    }

    #[test]
    fn prg_bank_bit4_disables_program_ram() {
        let mut mapper = new_mapper(0x20000, 0x8000, 1);

        write_program_ram(&mut mapper, 0x6B);
        assert_eq!(read_program_ram(&mapper), Some(0x6B));

        mapper.write_prog_bank(0x10);
        write_program_ram(&mut mapper, 0x80);
        assert_eq!(read_program_ram(&mapper), None);

        mapper.write_prog_bank(0x00);
        assert_eq!(read_program_ram(&mapper), Some(0x6B));
    }

    #[test]
    fn chr_bank_bit4_disables_program_ram_only_for_chr_ram_boards_up_to_256k_prg() {
        let mut snrom = new_mapper(0x20000, 0x0000, 1);
        write_program_ram(&mut snrom, 0x6B);
        snrom.write_char_bank_0(0x10);
        write_program_ram(&mut snrom, 0x80);
        snrom.write_char_bank_0(0x00);
        assert_eq!(read_program_ram(&snrom), Some(0x6B));

        let mut sxrom = new_mapper(0x80000, 0x0000, 1);
        write_program_ram(&mut sxrom, 0x6B);
        sxrom.write_char_bank_0(0x10);
        write_program_ram(&mut sxrom, 0x80);
        sxrom.write_char_bank_0(0x00);
        assert_eq!(read_program_ram(&sxrom), Some(0x80));
    }

    #[test]
    fn mapper_save_round_trips_persistent_prg_ram_only() {
        let data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x20000],
            char_rom: vec![0; 0x2000],
            pram_length: 0x2000,
            save_pram_length: 0x2000,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 1,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: true,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid");
        let mut source = SxRom::new(data.clone());
        Cartridge::initialize(&mut source);
        source.mapper_state_mut().sram[0] = 0xAA;
        source.mapper_state_mut().sram[0x2000] = 0xBB;

        let save = Cartridge::export_mapper_save_state(&source).expect("mapper save should export");
        assert_eq!(
            save,
            (
                vec![0xAA; 1]
                    .into_iter()
                    .chain(std::iter::repeat_n(0, 0x1FFF))
                    .collect(),
                Vec::new(),
            )
        );

        let mut target = SxRom::new(data);
        Cartridge::initialize(&mut target);
        Cartridge::import_mapper_save_state(&mut target, &save.0, &save.1)
            .expect("mapper save should import");
        assert_eq!(target.mapper_state_ref().sram[0], 0xAA);
        assert_eq!(target.mapper_state_ref().sram[0x2000], 0x00);
    }

    #[test]
    fn mapper_save_uses_legacy_ines_prg_ram_when_battery_backed() {
        let data = CartridgeData::new(CartridgeDataParts {
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
        let mut source = SxRom::new(data.clone());
        Cartridge::initialize(&mut source);
        source.mapper_state_mut().sram[0] = 0xCC;
        source.mapper_state_mut().sram[0x1FFF] = 0xDD;

        let save = Cartridge::export_mapper_save_state(&source).expect("mapper save should export");
        assert_eq!(save.0.len(), 0x2000);
        assert_eq!(save.0[0], 0xCC);
        assert_eq!(save.0[0x1FFF], 0xDD);
        assert!(save.1.is_empty());

        let mut target = SxRom::new(data);
        Cartridge::initialize(&mut target);
        Cartridge::import_mapper_save_state(&mut target, &save.0, &save.1)
            .expect("mapper save should import");
        assert_eq!(target.mapper_state_ref().sram[0], 0xCC);
        assert_eq!(target.mapper_state_ref().sram[0x1FFF], 0xDD);
    }
}
