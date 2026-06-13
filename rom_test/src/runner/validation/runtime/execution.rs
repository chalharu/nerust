use super::ValidationRuntime;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn run_frame(&mut self) -> u64 {
        let steps = self.core.run_frame(
            &mut self.screen_buffer,
            &mut self.controller,
            &mut self.mixer,
        );
        self.frame_counter += 1;
        steps
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    pub(in crate::runner::validation) fn reset(&mut self) {
        self.core.reset();
    }
}
