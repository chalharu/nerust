use crate::cartridge_data_parts::CartridgeDataParts;
use crate::cartridge_error::CartridgeError;
use nerust_contract_mirror::MirrorMode;
use nerust_contract_rom::RomFormat;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone)]
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
    use nerust_contract_mirror::MirrorMode;
    use serde::{Deserialize as _, Deserializer, Serializer, de::Error};
    use serde_derive::Deserialize;

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

#[cfg(test)]
mod tests {
    use super::{CartridgeData, RomFormat};
    use crate::Core;
    use crate::cartridge_data_parts::CartridgeDataParts;
    use nerust_contract_mirror::MirrorMode;

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
