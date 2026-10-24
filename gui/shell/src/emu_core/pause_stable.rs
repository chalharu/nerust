//! Host pause policy must not be overwritten by guest state restoration.
//! This adapter runs inside the worker, so restoration and re-parking are one operation:
//! a second queued Pause would leave a free-run frame between the two commands.
use nerust_core_traits::{ConsoleCore, CoreCapabilities, CoreConfig, CoreError};
use nerust_core_traits::{
    audio::StereoSample,
    debugger::{DebugControl, Debugger},
    identity::SystemIdentity,
};
use nerust_render_traits::FrameBuffer;

pub(super) struct PauseStableCore(pub(super) Box<dyn ConsoleCore>);

impl PauseStableCore {
    fn preserve_pause<R>(&mut self, operation: impl FnOnce(&mut dyn ConsoleCore) -> R) -> R {
        let paused = self.0.paused();
        let result = operation(self.0.as_mut());
        // Also restore on domain failure: imports may have partially mutated the core.
        self.0.set_paused(paused);
        result
    }
}

impl ConsoleCore for PauseStableCore {
    fn capabilities(&self) -> CoreCapabilities {
        self.0.capabilities()
    }
    fn render_frame(
        &mut self,
        frame: &mut FrameBuffer,
        audio: &mut Vec<StereoSample>,
    ) -> Result<(), CoreError> {
        self.0.render_frame(frame, audio)
    }
    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError> {
        self.0.load(rom, config)
    }
    fn unload(&mut self) {
        self.0.unload();
    }
    fn reset(&mut self) {
        self.preserve_pause(|core| core.reset());
    }
    fn paused(&self) -> bool {
        self.0.paused()
    }
    fn set_paused(&mut self, paused: bool) {
        self.0.set_paused(paused);
    }
    fn set_audio_sample_rate(&mut self, rate: u32) {
        self.0.set_audio_sample_rate(rate);
    }
    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        self.0.save_state()
    }
    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        self.preserve_pause(|core| core.load_state(data))
    }
    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        self.0.mapper_save()
    }
    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        self.preserve_pause(|core| core.import_mapper_save(data))
    }
    fn debugger(&self) -> Option<Box<dyn Debugger + '_>> {
        self.0.debugger()
    }
    fn debug_control(&mut self) -> Option<Box<dyn DebugControl + '_>> {
        self.0.debug_control()
    }
    fn output_channels(&self) -> Vec<String> {
        self.0.output_channels()
    }
    fn take_channel_bytes(&mut self, channel: &str) -> Vec<u8> {
        self.0.take_channel_bytes(channel)
    }
    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.0.identity()
    }
    fn rewind_state_size(&self) -> Option<usize> {
        self.0.rewind_state_size()
    }
    fn rewind_save(&self, data: &mut [u8]) {
        self.0.rewind_save(data);
    }
    fn rewind_restore(&mut self, data: &[u8]) {
        self.preserve_pause(|core| core.rewind_restore(data));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RestoresRunning {
        paused: bool,
    }
    impl ConsoleCore for RestoresRunning {
        fn capabilities(&self) -> CoreCapabilities {
            crate::test_helpers::build_test_core_parts()
                .core
                .capabilities()
        }
        fn render_frame(
            &mut self,
            _: &mut FrameBuffer,
            _: &mut Vec<StereoSample>,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        fn load(&mut self, _: &[u8], _: &CoreConfig) -> Result<(), CoreError> {
            Ok(())
        }
        fn unload(&mut self) {}
        fn reset(&mut self) {
            self.paused = false;
        }
        fn paused(&self) -> bool {
            self.paused
        }
        fn set_paused(&mut self, paused: bool) {
            self.paused = paused;
        }
        fn set_audio_sample_rate(&mut self, _: u32) {}
        fn save_state(&self) -> Result<Vec<u8>, CoreError> {
            Ok(vec![])
        }
        fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
            self.paused = false;
            if data.is_empty() {
                Ok(())
            } else {
                Err(CoreError::InvalidCoreOptions)
            }
        }
        fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
            self.load_state(data)
        }
    }

    #[test]
    fn restoration_preserves_host_pause_even_on_partial_failure() {
        for paused in [false, true] {
            let mut core = PauseStableCore(Box::new(RestoresRunning { paused }));
            core.load_state(&[]).unwrap();
            assert_eq!(core.paused(), paused);
            assert!(core.load_state(&[1]).is_err());
            assert_eq!(core.paused(), paused);
            assert!(core.import_mapper_save(&[1]).is_err());
            assert_eq!(core.paused(), paused);
            core.reset();
            assert_eq!(core.paused(), paused);
        }
    }
}
