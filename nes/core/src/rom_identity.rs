use crate::mirror::MirrorMode;
use crate::rom_format::RomFormat;
use nerust_contract_core::identity::SystemIdentity;
use nerust_contract_input::SystemId;

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
    pub fn into_system_identity(self) -> Result<SystemIdentity, String> {
        let identity_bytes = rmp_serde::to_vec_named(&self).map_err(|e| e.to_string())?;
        Ok(SystemIdentity::new(SystemId::new("nes"), identity_bytes))
    }
}
