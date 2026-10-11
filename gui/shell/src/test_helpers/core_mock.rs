use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError,
    audio::StereoSample,
    debugger::{
        DebugControl, DebugImage, Debugger, DebuggerError, DisasmLine, ImageFormat, SpaceAccess,
        SpaceId, SpaceInfo, SpaceTable, StepUnit,
    },
    identity::SystemIdentity,
};
use nerust_render_traits::{
    FrameBuffer, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};

use crate::test_helpers::{DummySystemId, test_input_resources};

/// Single 256-byte mock address space for debugger happy-path tests.
static MOCK_SPACES: [SpaceInfo; 1] = [SpaceInfo {
    id: SpaceId(0),
    key: "mock",
    name: "Mock",
    address_bits: 8,
    range: 0x00..=0xFF,
    access: SpaceAccess::ReadWrite,
}];

static MOCK_TABLE: SpaceTable = SpaceTable::build(&MOCK_SPACES);

pub(crate) struct MockConsoleCore {
    loaded: bool,
    paused: bool,
    identity: Option<SystemIdentity>,
    mem: [u8; 256],
    regs: [(&'static str, u64); 2],
}

impl MockConsoleCore {
    pub(crate) fn new() -> Self {
        Self {
            loaded: false,
            paused: true,
            identity: None,
            mem: [0; 256],
            regs: [("a", 1), ("b", 2)],
        }
    }
}

/// Read-only observer over the mock memory and registers.
pub(crate) struct MockDebugger<'a> {
    core: &'a MockConsoleCore,
}

impl Debugger for MockDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &MOCK_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        MOCK_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        if !MOCK_TABLE.covers(space, addr, width) {
            return None;
        }
        let mut value = 0u64;
        for i in 0..u32::from(width) {
            value |= u64::from(self.core.mem[(addr + i) as usize]) << (8 * i);
        }
        Some(value)
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        &self.core.regs
    }

    fn program_counter(&self) -> Option<u32> {
        Some(0x10)
    }

    fn disassemble(&self, addr: u32, count: u16) -> Vec<DisasmLine> {
        (0..count)
            .map(u32::from)
            .take_while(|i| addr + i <= 0xFF)
            .map(|i| {
                let at = addr + i;
                DisasmLine {
                    addr: at,
                    bytes: [0xEA, 0, 0, 0],
                    len: 1,
                    text: "NOP".to_string(),
                    is_pc: Some(at) == self.program_counter(),
                    target: None,
                }
            })
            .collect()
    }

    fn images(&self) -> Vec<DebugImage> {
        vec![DebugImage {
            id: "mock-tiles",
            label_id: "Mock tiles",
            width: 2,
            height: 1,
            format: ImageFormat::Indexed2bpp,
            palette: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]],
            pixels: vec![0, 3],
        }]
    }
}

/// Execution control over the mock memory and registers.
pub(crate) struct MockControl<'a> {
    core: &'a mut MockConsoleCore,
}

impl DebugControl for MockControl<'_> {
    fn step(&mut self, _unit: StepUnit) -> Result<u64, DebuggerError> {
        Ok(1)
    }

    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        value: u64,
    ) -> Result<(), DebuggerError> {
        MOCK_TABLE.check_write(space, addr, width)?;
        for i in 0..u32::from(width) {
            self.core.mem[(addr + i) as usize] = ((value >> (8 * i)) & 0xFF) as u8;
        }
        Ok(())
    }

    fn write_register(&mut self, name: &str, value: u64) -> Result<(), DebuggerError> {
        match name {
            "a" => self.core.regs[0].1 = value & 0xFF,
            "b" => self.core.regs[1].1 = value & 0xFF,
            _ => return Err(DebuggerError::Unsupported),
        }
        Ok(())
    }
}

impl ConsoleCore for MockConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: Vec::new(),
            video_signal: nerust_core_traits::VideoSignalKind::Ntsc,
        }
    }
    fn render_frame(
        &mut self,
        _frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<StereoSample>,
    ) -> Result<(), CoreError> {
        audio_out.clear();
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

    fn debugger(&self) -> Option<Box<dyn Debugger + '_>> {
        self.loaded
            .then(|| Box::new(MockDebugger { core: self }) as _)
    }

    fn debug_control(&mut self) -> Option<Box<dyn DebugControl + '_>> {
        self.loaded
            .then(|| Box::new(MockControl { core: self }) as _)
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
        audio: Box::new(nerust_core_traits::audio::NullAudio),
        gui_input,
        field_map: std::collections::HashMap::new(),
        render_profile,
        palette: Box::new([0u32; 256]),
        host_peripherals: Default::default(),
    }
}
