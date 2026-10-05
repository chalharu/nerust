use nerust_core_traits::audio::AudioBackend;

use super::ValidationRuntime;
use crate::{
    error::RomTestError,
    factory_adapter,
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

    /// Work-RAM byte through the generic debugger path.
    ///
    /// `space_containing` returning `None` is structurally impossible
    /// to mis-resolve: callers still treat `None` as out-of-range,
    /// exactly like the old concrete peek.
    pub(in crate::runner::validation) fn peek_work_ram(&self, address: usize) -> Option<u8> {
        let addr = u32::try_from(address).ok()?;
        let debugger = self.system.console.debugger()?;
        let space = debugger.space_containing(addr)?;
        debugger.read(space, addr, 1).map(|value| value as u8)
    }

    pub(in crate::runner::validation) fn peek_cartridge_ram(
        &self,
        address: usize,
    ) -> Option<(u8, bool)> {
        factory_adapter::peek_cartridge_ram(&*self.system.console, address)
    }

    pub(in crate::runner::validation) fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        let addr = u32::try_from(address).ok()?;
        let debugger = self.system.console.debugger()?;
        let space = debugger.space_containing(addr)?;
        debugger.read(space, addr, 1).map(|value| value as u8)
    }
}
