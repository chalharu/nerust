use super::ValidationRuntime;
use crate::error::RomTestError;
use nerust_core_traits::audio::AudioBackend;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn run_frame(&mut self) -> Result<(), RomTestError> {
        // Control borrows end before observation begins: stepping
        // provably cannot read taps, draining provably cannot steer.
        self.system.control().step_frame()?;
        let observe = self.system.observe();
        // Forward tapped nominal samples into the hashing mixer. The
        // stream is identical to direct pushing, so hashes are unchanged.
        for sample in observe.drain_audio()? {
            self.mixer.push(sample);
        }
        // Drained every frame (taps hold the latest delta only).
        // Keys were pre-registered at open (`get_mut`, no clones);
        // the channel list is a borrowed open-time snapshot.
        for channel in observe.channel_names() {
            let log = self.serial.get_mut(channel).ok_or_else(|| {
                RomTestError::EmuThread(format!(
                    "channel `{channel}` produced bytes without an open-time registration"
                ))
            })?;
            log.extend(observe.drain_channel(channel)?);
        }
        self.frame_counter += 1;
        Ok(())
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    pub(in crate::runner::validation) fn reset(&mut self) -> Result<(), RomTestError> {
        self.system.control().reset()?;
        // The core rebuilds its buffers on reset; drop every tap's
        // pre-reset delta and the cumulative logs together so
        // post-reset asserts observe fresh streams.
        let observe = self.system.observe();
        for channel in observe.channel_names() {
            drop(observe.drain_channel(channel)?);
        }
        self.serial.clear();
        Ok(())
    }
}
