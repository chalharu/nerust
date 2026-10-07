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
        for channel in self.system.channel_names().to_vec() {
            self.serial
                .entry(channel.clone())
                .or_default()
                .extend(self.system.drain_channel(&channel)?);
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
        for channel in self.system.channel_names().to_vec() {
            drop(self.system.drain_channel(&channel)?);
        }
        self.serial.clear();
        Ok(())
    }
}
