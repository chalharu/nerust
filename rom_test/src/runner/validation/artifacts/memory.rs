mod cartridge_ram;
mod ppu_vram;
mod work_ram;

#[derive(Default)]
pub(super) struct MemoryArtifacts {
    pub(super) work_ram: work_ram::WorkRamArtifacts,
    pub(super) cartridge_ram: cartridge_ram::CartridgeRamArtifacts,
    pub(super) ppu_vram: ppu_vram::PpuVramArtifacts,
}
