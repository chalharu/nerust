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
        screen_hash(self.system.observe().screen_buffer())
    }

    pub(in crate::runner::validation) fn capture_screenshot_png(
        &mut self,
    ) -> Result<Vec<u8>, RomTestError> {
        encode_screenshot_png(self.system.observe().screen_buffer())
    }

    /// Memory read through the thread inspect path: mapped value,
    /// open bus, or unmapped. The containing space resolves by
    /// address; callers interpret the outcome.
    pub(in crate::runner::validation) fn peek_memory(
        &self,
        address: usize,
    ) -> Result<crate::factory_adapter::MemoryRead, RomTestError> {
        let Some(addr) = u32::try_from(address).ok() else {
            return Ok(crate::factory_adapter::MemoryRead::Unmapped);
        };
        self.system.inspect().read_memory_byte(addr)
    }

    /// Live register list through the thread inspect path, in the
    /// debugger's ascending-name order.
    pub(in crate::runner::validation) fn peek_registers(
        &self,
    ) -> Result<Vec<(&'static str, u64)>, RomTestError> {
        self.system.inspect().read_registers()
    }

    /// Cumulative bytes produced on one channel since power-on (or
    /// the last reset). Grows only; unknown channels fail loudly
    /// (read-time validation against the drained set, like
    /// registers/spaces) — never silently empty.
    pub(in crate::runner::validation) fn peek_serial(
        &self,
        channel: &str,
    ) -> Result<&[u8], RomTestError> {
        self.serial.get(channel).map(Vec::as_slice).ok_or_else(|| {
            crate::error::RomTestError::EmuThread(format!(
                "unknown output channel `{channel}` (drained {:?})",
                self.serial.keys().collect::<Vec<_>>()
            ))
        })
    }
}
