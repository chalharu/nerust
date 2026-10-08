use super::ValidationRuntime;
use crate::error::RomTestError;
use nerust_core_traits::audio::AudioBackend;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn run_frame(&mut self) -> Result<(), RomTestError> {
        self.system.step_frame()?;
        // Forward tapped nominal samples into the hashing mixer. The
        // stream is identical to direct pushing, so hashes are unchanged.
        for sample in self.system.drain_audio()? {
            self.mixer.push(sample);
        }
        // Accumulate fresh bytes per channel into the cumulative
        // logs. Drained every frame so multi-frame gaps between
        // asserts lose nothing (each tap holds the latest delta only).
        // The channel set is fixed at open (`TestSystem.channels`
        // snapshot), so iterating the borrowed list costs nothing per
        // frame — no re-query, no clone of the name list. The per-frame
        // `Vec` from `drain_*` is likewise negligible (measured ~1µs
        // against ~140µs+ of framebuffer hashing alone, before core
        // emulation), so no `drain_into` streaming API is warranted.
        for channel in self.system.channel_names() {
            self.serial
                .entry(channel.clone())
                .or_default()
                .extend(self.system.drain_channel(channel)?);
        }
        self.frame_counter += 1;
        Ok(())
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    pub(in crate::runner::validation) fn reset(&mut self) -> Result<(), RomTestError> {
        self.system.reset()?;
        // The core rebuilds its buffers on reset; drop every tap's
        // pre-reset delta and the cumulative logs together so
        // post-reset asserts observe fresh streams.
        for channel in self.system.channel_names() {
            drop(self.system.drain_channel(channel)?);
        }
        self.serial.clear();
        Ok(())
    }
}
