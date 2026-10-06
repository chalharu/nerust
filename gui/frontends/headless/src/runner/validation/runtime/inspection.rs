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

    pub(in crate::runner::validation) fn screen_hash(&mut self) -> u64 {
        screen_hash(self.system.screen_buffer())
    }

    pub(in crate::runner::validation) fn capture_screenshot_png(
        &mut self,
    ) -> Result<Vec<u8>, RomTestError> {
        encode_screenshot_png(self.system.screen_buffer())
    }

    /// Work-RAM byte through the thread inspect path.
    pub(in crate::runner::validation) fn peek_work_ram(
        &self,
        address: usize,
    ) -> Result<Option<u8>, RomTestError> {
        let Some(addr) = u32::try_from(address).ok() else {
            return Ok(None);
        };
        self.system.read_memory_byte(addr)
    }

    pub(in crate::runner::validation) fn peek_cartridge_ram(
        &self,
        address: usize,
    ) -> Result<Option<(u8, bool)>, RomTestError> {
        self.system.peek_cartridge_ram(address)
    }

    pub(in crate::runner::validation) fn peek_ppu_vram(
        &self,
        address: usize,
    ) -> Result<Option<u8>, RomTestError> {
        let Some(addr) = u32::try_from(address).ok() else {
            return Ok(None);
        };
        self.system.read_memory_byte(addr)
    }
}
