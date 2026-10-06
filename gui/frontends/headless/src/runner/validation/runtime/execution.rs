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
        // Accumulate fresh serial bytes into the cumulative log.
        // Drained every frame so multi-frame gaps between asserts lose
        // nothing (the tap holds the latest delta only).
        self.serial.extend(self.system.drain_serial()?);
        self.frame_counter += 1;
        Ok(())
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    pub(in crate::runner::validation) fn reset(&mut self) -> Result<(), RomTestError> {
        self.system.reset()?;
        // The core rebuilds its serial buffer on reset; drop the tap's
        // pre-reset delta and the cumulative log together so post-reset
        // asserts observe a fresh stream.
        drop(self.system.drain_serial()?);
        self.serial.clear();
        Ok(())
    }
}
