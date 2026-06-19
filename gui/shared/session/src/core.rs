use nerust_console::state::RuntimeStateExport;
use nerust_console::video::{ConsoleVideo, VideoFrameHandle, VideoRenderProfile};
use nerust_console::{Console, ConsoleError, ConsoleMetrics};
use nerust_contract_core::options::CoreOptions;
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_screen_video::FrameBuffer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

pub struct SessionCore {
    paused: bool,
    loaded: bool,
    console: Console,
    window_size: WindowSize,
}

impl SessionCore {
    pub fn from_console(console: Console) -> Self {
        let metrics = console.metrics();
        let physical_size = console.video().render_profile().physical_size;
        let window_size = WindowSize {
            width: physical_size.width,
            height: physical_size.height,
        };
        Self {
            paused: metrics.paused,
            loaded: metrics.loaded,
            console,
            window_size,
        }
    }

    pub fn video(&self) -> &ConsoleVideo {
        self.console.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.console.with_frame_buffer(f)
    }

    pub fn window_size(&self) -> WindowSize {
        self.window_size
    }

    pub fn video_frame_handle(&self) -> VideoFrameHandle {
        use std::sync::Arc;
        let profile = self.console.video().render_profile();
        let logical_w = profile.logical_size.width;
        let bpp = profile.frame_format.bytes_per_pixel();
        self.console.with_frame_buffer(|bytes| VideoFrameHandle {
            width: logical_w as u32,
            height: profile.logical_size.height as u32,
            stride_bytes: logical_w * bpp,
            bytes: Arc::from(bytes),
        })
    }

    pub fn swap_frame_buffer(&mut self) -> bool {
        self.console.swap_frame_buffer()
    }

    pub fn frame_buffer(&self) -> &FrameBuffer {
        self.console.frame_buffer()
    }

    pub fn video_render_profile(&self) -> VideoRenderProfile {
        self.console.video().render_profile()
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.console.metrics()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn can_pause(&self) -> bool {
        self.loaded && !self.paused
    }

    pub fn can_resume(&self) -> bool {
        self.loaded && self.paused
    }

    pub fn reset(&self) -> Result<(), ConsoleError> {
        self.console.reset()
    }

    pub fn pause(&mut self) {
        self.console.pause();
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.console.resume();
        self.paused = false;
    }

    pub fn load_rom(&mut self, data: Vec<u8>, options: CoreOptions) -> Result<(), ConsoleError> {
        self.console.load_with_options(data, options)?;
        self.loaded = true;
        Ok(())
    }

    pub fn unload_rom(&mut self) -> Result<(), ConsoleError> {
        let result = self.console.unload();
        self.loaded = false;
        result
    }

    pub fn export_state(&self) -> Result<RuntimeStateExport, ConsoleError> {
        self.console.export_state()
    }

    pub fn import_state(&mut self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.console.import_state(bytes)
    }

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, ConsoleError> {
        self.console.export_mapper_save()
    }

    pub fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.console.import_mapper_save(bytes)
    }

    pub fn canonical_media_identity(&self) -> Result<CanonicalMediaIdentity, ConsoleError> {
        self.console.canonical_media_identity()
    }

    pub fn set_volume(&self, volume: f32) {
        self.console.set_volume(volume);
    }

    pub fn sync_paused_from_console(&mut self) {
        self.paused = self.console.metrics().paused;
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCore;
    use nerust_console::Console;
    use nerust_contract_core::audio::AudioBackend;

    #[derive(Default)]
    struct TestSpeaker;

    impl AudioBackend for TestSpeaker {
        fn start(&mut self) {}
        fn pause(&mut self) {}
        fn push(&mut self, _data: f32) {}
    }

    fn test_core() -> SessionCore {
        use std::sync::Arc;
        SessionCore::from_console(Console::new_gpu(
            Box::new(TestSpeaker),
            nerust_screen_video::FilterType::NtscComposite,
            nerust_screen_video::LogicalSize {
                width: 256,
                height: 240,
            },
            Box::new(nerust_nes_device::nes_pad::NesPadDevice::new(
                nerust_input_nes_runtime::nes_input_cell::SharedNesInputCell(Arc::new(
                    nerust_input_nes_runtime::nes_input_cell::NesInputCell::new(),
                )),
            )),
        ))
    }

    #[test]
    fn session_core_from_console_preserves_video_shape() {
        let core = test_core();

        assert!(core.paused());
        assert!(!core.loaded());
        assert!(!core.can_pause());
        assert!(!core.can_resume());
        assert_eq!(
            core.video().render_profile().physical_size.width,
            core.window_size().width
        );
        assert!(!core.video_frame_handle().bytes().is_empty());
    }
}
