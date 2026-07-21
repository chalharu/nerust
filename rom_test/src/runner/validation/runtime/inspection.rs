use nerust_core_traits::{audio::AudioBackend, debugger::Debugger as _};
use nerust_nes_core::debugger::memory_space::NesMemorySpace;

use super::ValidationRuntime;
use crate::{
    error::RomTestError,
    media::{encode_screenshot_png, screen_hash},
};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn audio_sample_rate(&self) -> u32 {
        self.mixer.sample_rate()
    }

    pub(in crate::runner::validation) fn audio_samples(&self) -> u64 {
        self.mixer.samples()
    }

    pub(in crate::runner::validation) fn audio_hash(&self) -> u64 {
        self.mixer.checksum()
    }

    pub(in crate::runner::validation) fn screen_hash(&self) -> u64 {
        screen_hash(&self.screen_buffer)
    }

    pub(in crate::runner::validation) fn capture_screenshot_png(
        &self,
    ) -> Result<Vec<u8>, RomTestError> {
        encode_screenshot_png(&self.screen_buffer)
    }

    pub(in crate::runner::validation) fn peek_work_ram(&self, address: usize) -> Option<u8> {
        self.debugger.read(&NesMemorySpace::Cpu, address as u32)
    }

    pub(in crate::runner::validation) fn peek_cartridge_ram(
        &self,
        address: usize,
    ) -> Option<(u8, bool)> {
        self.debugger.read_cartridge_ram(address as u16)
    }

    pub(in crate::runner::validation) fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        self.debugger.read(&NesMemorySpace::Ppu, address as u32)
    }
}
