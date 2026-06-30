use nerust_core_traits::audio::AudioBackend;

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
        self.core.peek_work_ram(address)
    }

    pub(in crate::runner::validation) fn peek_cartridge_ram(
        &self,
        address: usize,
    ) -> Option<(u8, bool)> {
        self.core
            .peek_cartridge_ram(address)
            .map(|read_result| (read_result.data, read_result.mask != 0xFF))
    }

    pub(in crate::runner::validation) fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        self.core.peek_ppu_vram(address)
    }
}
