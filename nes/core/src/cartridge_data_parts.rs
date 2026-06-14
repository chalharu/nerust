use nerust_contract_core::mirror::MirrorMode;
use nerust_contract_core::rom::RomFormat;

#[derive(Debug, Clone)]
pub struct CartridgeDataParts {
    pub format: RomFormat,
    pub prog_rom: Vec<u8>,
    pub char_rom: Vec<u8>,
    pub pram_length: usize,
    pub save_pram_length: usize,
    pub vram_length: usize,
    pub save_vram_length: usize,
    pub mapper_type: u16,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub sub_mapper_type: u8,
    pub trainer: Vec<u8>,
}
