use super::ValidationRuntime;
use crate::error::RomTestError;
use nerust_core_traits::audio::AudioBackend;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn run_frame(&mut self) -> Result<(), RomTestError> {
        self.audio_sink.clear();
        self.system
            .console
            .render_frame(&mut self.screen_buffer, &mut self.audio_sink)
            .map_err(|error| RomTestError::RenderFrame(error.to_string()))?;
        // Forward production audio into the hashing mixer. The sample
        // stream is identical to direct pushing, so hashes are unchanged.
        for sample in self.audio_sink.drain(..) {
            self.mixer.push(sample);
        }
        self.frame_counter += 1;
        Ok(())
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    pub(in crate::runner::validation) fn reset(&mut self) {
        self.system.console.reset();
    }
}
