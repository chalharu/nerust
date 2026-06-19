use crate::cartridge_data_parts::CartridgeDataParts;
use crate::cartridge_error::CartridgeError;
use nerust_contract_core::mirror::MirrorMode;
use nerust_contract_core::rom::RomFormat;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct CartridgeData {
    format: RomFormat,
    #[serde(with = "serde_bytes")]
    prog_rom: Vec<u8>,
    #[serde(with = "serde_bytes")]
    char_rom: Vec<u8>,
    pram_length: usize,
    save_pram_length: usize,
    vram_length: usize,
    save_vram_length: usize,
    mapper_type: u16,
    #[serde(with = "mirror_mode_serde")]
    mirror_mode: MirrorMode,
    has_battery: bool,
    sub_mapper_type: u8,
    #[serde(default)]
    #[serde(with = "serde_bytes")]
    trainer: Vec<u8>,
}

impl CartridgeData {
    pub fn new(parts: CartridgeDataParts) -> Result<Self, CartridgeError> {
        let data = Self {
            format: parts.format,
            prog_rom: parts.prog_rom,
            char_rom: parts.char_rom,
            pram_length: parts.pram_length,
            save_pram_length: parts.save_pram_length,
            vram_length: parts.vram_length,
            save_vram_length: parts.save_vram_length,
            mapper_type: parts.mapper_type,
            mirror_mode: parts.mirror_mode,
            has_battery: parts.has_battery,
            sub_mapper_type: parts.sub_mapper_type,
            trainer: parts.trainer,
        };
        data.validate()?;
        Ok(data)
    }

    pub fn validate(&self) -> Result<(), CartridgeError> {
        if self.sub_mapper_type > 0x0F {
            return Err(CartridgeError::DataError);
        }

        match self.mirror_mode {
            MirrorMode::Horizontal
            | MirrorMode::Vertical
            | MirrorMode::Single0
            | MirrorMode::Single1
            | MirrorMode::Four => {}
            MirrorMode::Custom(_) => return Err(CartridgeError::DataError),
        }

        if self.prog_rom.len() < 0x4000 {
            return Err(CartridgeError::DataError);
        }

        if !self.char_rom.is_empty() && self.char_rom.len() < 0x0100 {
            return Err(CartridgeError::DataError);
        }

        Ok(())
    }

    pub fn format(&self) -> RomFormat {
        self.format
    }

    pub fn mapper_type(&self) -> u16 {
        self.mapper_type
    }

    pub fn sub_mapper_type(&self) -> u8 {
        self.sub_mapper_type
    }

    pub fn read_prog_rom(&self, index: usize) -> u8 {
        self.prog_rom[index]
    }

    pub fn prog_rom(&self) -> &[u8] {
        &self.prog_rom
    }

    pub fn read_char_rom(&self, index: usize) -> u8 {
        self.char_rom[index]
    }

    pub fn char_rom(&self) -> &[u8] {
        &self.char_rom
    }

    pub fn prog_rom_len(&self) -> usize {
        self.prog_rom.len()
    }

    pub fn char_rom_len(&self) -> usize {
        self.char_rom.len()
    }

    pub fn write_prog_rom(&mut self, index: usize, data: u8) {
        self.prog_rom[index] = data;
    }

    pub fn mirror_mode(&self) -> MirrorMode {
        self.mirror_mode
    }

    pub fn pram_length(&self) -> usize {
        self.pram_length
    }

    pub fn save_pram_length(&self) -> usize {
        self.save_pram_length
    }

    pub fn vram_length(&self) -> usize {
        self.vram_length
    }

    pub fn save_vram_length(&self) -> usize {
        self.save_vram_length
    }

    pub fn has_battery(&self) -> bool {
        self.has_battery
    }

    pub fn trainer(&self) -> &[u8] {
        &self.trainer
    }
}

mod mirror_mode_serde {
    use nerust_contract_core::mirror::MirrorMode;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MirrorModeRepr {
        Raw(u8),
        Legacy(MirrorMode),
    }

    pub(super) fn serialize<S>(mode: &MirrorMode, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(match mode {
            MirrorMode::Horizontal => 0,
            MirrorMode::Vertical => 1,
            MirrorMode::Single0 => 2,
            MirrorMode::Single1 => 3,
            MirrorMode::Four => 4,
            MirrorMode::Custom(lut) => {
                return Err(serde::ser::Error::custom(format!(
                    "unsupported custom mirror mode: {lut:?}"
                )));
            }
        })
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<MirrorMode, D::Error>
    where
        D: Deserializer<'de>,
    {
        match MirrorModeRepr::deserialize(deserializer)? {
            MirrorModeRepr::Raw(mode) => {
                MirrorMode::try_from(mode).map_err(|message| D::Error::custom(message.to_string()))
            }
            MirrorModeRepr::Legacy(mode) => match mode {
                MirrorMode::Custom([0, 0, 1, 1]) => Ok(MirrorMode::Horizontal),
                MirrorMode::Custom([0, 1, 0, 1]) => Ok(MirrorMode::Vertical),
                MirrorMode::Custom([0, 0, 0, 0]) => Ok(MirrorMode::Single0),
                MirrorMode::Custom([1, 1, 1, 1]) => Ok(MirrorMode::Single1),
                MirrorMode::Custom([0, 1, 2, 3]) => Ok(MirrorMode::Four),
                MirrorMode::Custom(lut) => Err(D::Error::custom(format!(
                    "unsupported custom mirror mode: {lut:?}"
                ))),
                mode => Ok(mode),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// ROM パース: iNES / NES 2.0
// ---------------------------------------------------------------------------
use std::cmp;

pub fn parse_rom(data: &[u8]) -> Result<CartridgeData, CartridgeError> {
    if data.len() < 16 {
        return Err(CartridgeError::UnexpectedEof);
    }
    if data[0] != 0x4E || data[1] != 0x45 || data[2] != 0x53 || data[3] != 0x1A {
        return Err(CartridgeError::DataError);
    }

    let flags2 = data[7];

    if flags2 & 0x0C == 0x08 {
        parse_nes20(data)
    } else {
        parse_ines(data)
    }
}

fn parse_ines(data: &[u8]) -> Result<CartridgeData, CartridgeError> {
    let prom_length = usize::from(data[4]) * 0x4000;
    let crom_length = usize::from(data[5]) * 0x2000;
    let flags1 = data[6];
    let flags2 = data[7];
    let pram_length = cmp::max(usize::from(data[8]), 1) * 0x2000;

    let mapper_type = u16::from((flags1 >> 4) | (flags2 & 0xf0));
    let mirror_bits = (flags1 & 1) | ((flags1 >> 2) & 2);
    let mirror_mode = MirrorMode::try_from(mirror_bits).map_err(|_| CartridgeError::DataError)?;
    let has_battery = (flags1 & 2) == 2;
    let has_trainer = (flags1 & 4) == 4;

    let mut offset = 16;
    let trainer = if has_trainer {
        let end = offset + 512;
        if end > data.len() {
            return Err(CartridgeError::UnexpectedEof);
        }
        let t = data[offset..end].to_vec();
        offset = end;
        t
    } else {
        Vec::new()
    };

    let prog_end = offset + prom_length;
    if prog_end > data.len() {
        return Err(CartridgeError::UnexpectedEof);
    }
    let prog_rom = data[offset..prog_end].to_vec();
    offset = prog_end;

    let char_rom = if crom_length != 0 {
        let chr_end = offset + crom_length;
        if chr_end > data.len() {
            return Err(CartridgeError::UnexpectedEof);
        }
        data[offset..chr_end].to_vec()
    } else {
        Vec::new()
    };

    let vram_length = if crom_length != 0 { 0 } else { 0x2000 };

    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom,
        char_rom,
        pram_length,
        save_pram_length: 0,
        vram_length,
        save_vram_length: 0,
        mapper_type,
        mirror_mode,
        has_battery,
        sub_mapper_type: 0,
        trainer,
    })
}

fn parse_nes20(data: &[u8]) -> Result<CartridgeData, CartridgeError> {
    let upper_rom_size = usize::from(data[9]);
    let prom_length = (usize::from(data[4]) | ((upper_rom_size & 0x0F) << 8)) * 0x4000;
    let crom_length = (usize::from(data[5]) | ((upper_rom_size & 0xF0) << 4)) * 0x2000;
    let flags1 = data[6];
    let flags2 = data[7];
    let mapper_variant = data[8];
    let pram_length_data = usize::from(data[10]);
    let vram_length_data = usize::from(data[11]);
    let pram_length_shift = pram_length_data & 0x0F;
    let save_pram_length_shift = pram_length_data >> 4;
    let vram_length_shift = vram_length_data & 0x0F;
    let save_vram_length_shift = vram_length_data >> 4;

    let pram_length = if pram_length_shift == 0 {
        0
    } else {
        1 << (6 + pram_length_shift)
    };
    let save_pram_length = if save_pram_length_shift == 0 {
        0
    } else {
        1 << (6 + save_pram_length_shift)
    };
    let vram_length = if vram_length_shift == 0 {
        if crom_length == 0 && save_vram_length_shift == 0 {
            0x2000
        } else {
            0
        }
    } else {
        1 << (6 + vram_length_shift)
    };
    let save_vram_length = if save_vram_length_shift == 0 {
        0
    } else {
        1 << (6 + save_vram_length_shift)
    };

    let mapper_type =
        u16::from(flags1 >> 4) | u16::from(flags2 & 0xf0) | (u16::from(mapper_variant & 0x0F) << 8);
    let sub_mapper_type = mapper_variant >> 4;
    let mirror_bits = (flags1 & 1) | ((flags1 >> 2) & 2);
    let mirror_mode = MirrorMode::try_from(mirror_bits).map_err(|_| CartridgeError::DataError)?;
    let has_battery = (flags1 & 2) == 2;
    let has_trainer = (flags1 & 4) == 4;

    let mut offset = 16;
    let trainer = if has_trainer {
        let end = offset + 512;
        if end > data.len() {
            return Err(CartridgeError::UnexpectedEof);
        }
        let t = data[offset..end].to_vec();
        offset = end;
        t
    } else {
        Vec::new()
    };

    let prog_end = offset + prom_length;
    if prog_end > data.len() {
        return Err(CartridgeError::UnexpectedEof);
    }
    let prog_rom = data[offset..prog_end].to_vec();
    offset = prog_end;

    let char_rom = if crom_length != 0 {
        let chr_end = offset + crom_length;
        if chr_end > data.len() {
            return Err(CartridgeError::UnexpectedEof);
        }
        data[offset..chr_end].to_vec()
    } else {
        Vec::new()
    };

    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::Nes20,
        prog_rom,
        char_rom,
        pram_length,
        save_pram_length,
        vram_length,
        save_vram_length,
        mapper_type,
        mirror_mode,
        has_battery,
        sub_mapper_type,
        trainer,
    })
}

#[cfg(test)]
mod tests {
    use super::{CartridgeData, RomFormat};
    use crate::Core;
    use crate::cartridge_data_parts::CartridgeDataParts;
    use nerust_contract_core::mirror::MirrorMode;

    #[test]
    fn inspect_cartridge_reads_ines_metadata() {
        let cartridge_data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0x2000,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 4,
            mirror_mode: MirrorMode::Vertical,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid");

        let info = Core::inspect_cartridge(&cartridge_data, 16 + 0x8000 + 0x2000)
            .expect("rom info should inspect");

        assert_eq!(info.format, RomFormat::INes);
        assert_eq!(info.mapper_type, 4);
        assert_eq!(info.sub_mapper_type, 0);
        assert_eq!(info.mirror_mode, MirrorMode::Vertical);
        assert!(!info.has_battery);
        assert_eq!(info.trainer_len, 0);
        assert_eq!(info.prg_rom_len, 0x8000);
        assert_eq!(info.chr_rom_len, 0x2000);
        assert_eq!(info.prg_ram_len, 0x2000);
        assert_eq!(info.save_prg_ram_len, 0);
        assert_eq!(info.chr_ram_len, 0);
        assert_eq!(info.save_chr_ram_len, 0);
        assert_eq!(info.raw_file_len, 16 + 0x8000 + 0x2000);
        assert_eq!(info.body_len, 0x8000 + 0x2000);
    }

    #[test]
    fn inspect_cartridge_reports_effective_legacy_battery_save_ram_length() {
        let cartridge_data = CartridgeData::new(CartridgeDataParts {
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

        let info =
            Core::inspect_cartridge(&cartridge_data, 16 + 0x20000 + 0x2000).expect("inspect");

        assert_eq!(info.prg_ram_len, 0x2000);
        assert_eq!(info.save_prg_ram_len, 0x2000);
    }

    #[test]
    fn rejects_too_small_program_rom() {
        let result = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x2000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        });

        assert!(result.is_err());
    }

    #[test]
    fn core_rejects_invalid_geometry_even_if_data_is_constructed_directly() {
        let cartridge_data = CartridgeData {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x2000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        };

        assert!(Core::new(cartridge_data).is_err());
    }
}
