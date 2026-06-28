use nerust_core_traits::identity::SystemIdentity;
use nerust_input_traits::SystemId;

use crate::{mirror::MirrorMode, rom_format::RomFormat};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomIdentity {
    pub format: RomFormat,
    pub mapper_type: u16,
    pub sub_mapper_type: u8,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub trainer_len: usize,
    pub prg_rom_len: usize,
    pub chr_rom_len: usize,
    pub prg_ram_len: usize,
    pub save_prg_ram_len: usize,
    pub chr_ram_len: usize,
    pub save_chr_ram_len: usize,
    pub prg_rom_crc64: u64,
    pub chr_rom_crc64: u64,
    pub trainer_crc64: u64,
}

impl RomIdentity {
    pub fn into_system_identity(self) -> Result<SystemIdentity, rmp_serde::encode::Error> {
        let identity_bytes = rmp_serde::to_vec_named(&self)?;
        Ok(SystemIdentity::new(SystemId::new("nes"), identity_bytes))
    }
}
