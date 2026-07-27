use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, identity::SystemIdentity,
};
use nerust_render_traits::{
    FrameBuffer, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};

use crate::test_helpers::{DummySystemId, test_input_resources};

pub(crate) struct MockConsoleCore {
    loaded: bool,
    paused: bool,
    identity: Option<SystemIdentity>,
}

impl MockConsoleCore {
    pub(crate) fn new() -> Self {
        Self {
            loaded: false,
            paused: true,
            identity: None,
        }
    }
}

impl ConsoleCore for MockConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: Vec::new(),
            video_signal: nerust_core_traits::VideoSignalKind::Ntsc,
        }
    }
    fn render_frame(&mut self, _frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        Ok(())
    }
    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        self.loaded = true;
        self.paused = true;
        self.identity = Some(SystemIdentity::new(
            Box::new(DummySystemId),
            rom.get(6..8).unwrap_or(&[0, 0]).to_vec(),
        ));
        Ok(())
    }
    fn unload(&mut self) {
        self.loaded = false;
    }
    fn reset(&mut self) {}
    fn paused(&self) -> bool {
        self.paused
    }
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        Ok(vec![])
    }
    fn load_state(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.identity.clone().ok_or(CoreError::NoRomLoaded)
    }
}

pub(crate) fn build_test_core_parts() -> nerust_core_traits::factory::CoreParts {
    use nerust_core_traits::factory::CoreParts;
    let core = MockConsoleCore::new();
    let render_profile = VideoRenderProfile {
        source_logical_size: LogicalSize {
            width: 256,
            height: 240,
        },
        logical_size: LogicalSize {
            width: 256,
            height: 240,
        },
        physical_size: PhysicalSize {
            width: 256.0,
            height: 240.0,
        },
        frame_format: nerust_render_traits::VideoFrameFormat::Palette,
        ntsc_packed_rgba8: None,
    };
    let (gui_input, _input_split) = test_input_resources();
    CoreParts {
        core: Box::new(core),
        gui_input,
        field_map: std::collections::HashMap::new(),
        render_profile,
        palette: Box::new([0u32; 256]),
    }
}
