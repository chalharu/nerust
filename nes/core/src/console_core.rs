use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind,
    audio::{AudioBackend, StereoSample},
    debugger::{DebugControl, DebuggerError, SpaceAccess, SpaceId, SpaceTable, StepUnit},
    identity::SystemIdentity,
};
use nerust_input_traits::{ControllerCollection, ControllerHub as _, EmuInput};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{
    Core, cartridge_rom::CartridgeData, core_options::CoreOptions, debugger::NES_SPACE_TABLE,
    input_types::NesInputBuffer,
};

/// `Core` は `pub(crate)` な `Cartridge` trait (`Box<dyn Cartridge>`) を含む。
/// 全ての具象 mapper は同一 crate 内 (`nes/core/src/cartridge/mapper/`) にあり、
/// かつ全て Send であることが確認されているため、`unsafe impl Send` は安全。
struct SendCore(Option<Core>);

// Safety: 全ての Cartridge 実装は同一 crate 内にあり、全て Send。
// `pub(crate)` なので外部からの非 Send 実装追加は不可能。
unsafe impl Send for SendCore {}

pub struct NesConsoleCore {
    core: SendCore,
    controller: ControllerCollection,
    emu_input: EmuInput,
    paused: bool,
    /// Device sample rate for the APU resampler, stamped from
    /// `CoreConfig::audio_sample_rate` at load (data only; the backend
    /// lives in the session layer).
    sample_rate: u32,
}

/// Session-side audio sink: collects the frame's nominal samples into
/// the caller-provided Vec (transport separation: the core never sees
/// the real backend).
struct VecSink<'a> {
    out: &'a mut Vec<StereoSample>,
    sample_rate: u32,
}

impl AudioBackend for VecSink<'_> {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, sample: StereoSample) {
        self.out.push(sample);
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

impl NesConsoleCore {
    pub fn new(
        cartridge_data: CartridgeData,
        controller: ControllerCollection,
        emu_input: EmuInput,
    ) -> Result<Self, CoreError> {
        let core = Core::new(cartridge_data).map_err(CoreError::Core)?;
        Ok(Self {
            core: SendCore(Some(core)),
            controller,
            emu_input,
            paused: false,
            sample_rate: 48_000,
        })
    }

    /// Creates a NesConsoleCore with no ROM loaded.
    /// Call `load()` before `render_frame()`.
    pub fn new_empty(controller: ControllerCollection, emu_input: EmuInput) -> Self {
        Self {
            core: SendCore(None),
            controller,
            emu_input,
            paused: false,
            sample_rate: 48_000,
        }
    }
}

impl NesConsoleCore {
    pub(crate) fn core_ref(&self) -> Result<&Core, CoreError> {
        self.core.0.as_ref().ok_or(CoreError::NoRomLoaded)
    }

    pub(crate) fn core_mut(&mut self) -> Result<&mut Core, CoreError> {
        self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)
    }

    /// Advance one frame, reporting the cycles the core counted.
    ///
    /// Same path as the [`ConsoleCore::render_frame`] implementation;
    /// the cycle count that path discards is returned here for
    /// debugger stepping.
    pub(crate) fn render_frame_cycles(
        &mut self,
        frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<StereoSample>,
    ) -> Result<u64, CoreError> {
        let core = self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)?;

        // Take latest input and sync to controller
        self.emu_input.take();
        if let Some(state) = self.emu_input.read_buf.downcast_ref::<NesInputBuffer>() {
            self.controller.sync_input(&state.0);
        }

        // Nominal-rate audio production only: samples collect into the
        // caller buffer; the session layer owns transport to the backend.
        audio_out.clear();
        let mut sink = VecSink {
            out: audio_out,
            sample_rate: self.sample_rate,
        };
        let cycles = core.run_frame(frame_slot, &mut self.controller, &mut sink);

        Ok(cycles)
    }
}

impl ConsoleCore for NesConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            }],
            video_signal: VideoSignalKind::Ntsc,
        }
    }

    fn render_frame(
        &mut self,
        frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<StereoSample>,
    ) -> Result<(), CoreError> {
        self.render_frame_cycles(frame_slot, audio_out)?;
        Ok(())
    }

    // `region` フィールドは NES PAL 対応時に使用する。
    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError> {
        let cartridge_data =
            crate::rom_parse::parse_rom(rom).map_err(|e| CoreError::RomParse(Box::new(e)))?;
        let options = if let Some(core_options) = &config.core_options {
            *core_options
                .clone()
                .downcast::<CoreOptions>()
                .map_err(|_| CoreError::InvalidCoreOptions)?
        } else {
            CoreOptions::default()
        };
        let core = Core::new_with_options(cartridge_data, options).map_err(CoreError::Core)?;
        self.core = SendCore(Some(core));
        self.paused = false;
        self.sample_rate = config.audio_sample_rate.unwrap_or(48_000);
        Ok(())
    }

    fn unload(&mut self) {
        self.core = SendCore(None);
        self.paused = false;
    }

    fn reset(&mut self) {
        if let Some(core) = self.core.0.as_mut() {
            core.reset();
        }
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let core = self.core_ref()?;
        core.export_machine_state().map_err(CoreError::Core)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let core = self.core_mut()?;
        core.import_machine_state(data).map_err(CoreError::Core)
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let core = self.core_ref()?;
        core.export_mapper_save().map_err(CoreError::Core)
    }

    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let core = self.core_mut()?;
        core.import_mapper_save(data).map_err(CoreError::Core)
    }

    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.core_ref()?
            .rom_identity()
            .into_system_identity()
            .map_err(|e| CoreError::Core(Box::new(e)))
    }

    fn debugger(&self) -> Option<Box<dyn nerust_core_traits::debugger::Debugger + '_>> {
        self.core
            .0
            .as_ref()
            .map(|core| Box::new(crate::debugger::NesDebugger::new(core)) as _)
    }

    fn debug_control(
        &mut self,
    ) -> Option<Box<dyn nerust_core_traits::debugger::DebugControl + '_>> {
        self.core.0.as_mut()?;
        Some(Box::new(NesDebugControl::new(self)) as _)
    }
}

/// NES execution control and memory editing.
///
/// Holds the console (not just the core): frame stepping reuses the
/// exact `render_frame` path and reports the cycles it counted.
/// Scratch buffers are display-irrelevant (zeroed palette); stepped
/// frames are not published to any shared framebuffer here.
pub struct NesDebugControl<'a> {
    console: &'a mut NesConsoleCore,
    frame_slot: FrameBuffer,
    audio_sink: Vec<StereoSample>,
}

impl<'a> NesDebugControl<'a> {
    pub fn new(console: &'a mut NesConsoleCore) -> Self {
        let mut frame_slot = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        );
        frame_slot.resize(256, 240);
        Self {
            console,
            frame_slot,
            audio_sink: Vec::new(),
        }
    }
}

impl DebugControl for NesDebugControl<'_> {
    fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError> {
        match unit {
            StepUnit::Frame => self
                .console
                .render_frame_cycles(&mut self.frame_slot, &mut self.audio_sink)
                .map_err(|_| DebuggerError::Unsupported),
            // Instruction stepping is a required debug capability, but
            // needs an instruction-boundary API in the CPU core first.
            StepUnit::Instruction => Err(DebuggerError::UnsupportedStepUnit(unit)),
        }
    }

    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        value: u64,
    ) -> Result<(), DebuggerError> {
        // Validation order matters: bad width first, so "width 0 with
        // unknown SpaceId" does not mask as UnknownSpace.
        if !SpaceTable::width_is_valid(width) {
            return Err(DebuggerError::BadWidth(width));
        }
        let info = NES_SPACE_TABLE
            .get(space)
            .ok_or(DebuggerError::UnknownSpace(space))?;
        if !NES_SPACE_TABLE.covers(space, addr, width) {
            return Err(DebuggerError::UnmappedAddress { space, addr });
        }
        if info.access == SpaceAccess::ReadOnly {
            return Err(DebuggerError::ReadOnlySpace(space));
        }
        let core = self
            .console
            .core_mut()
            .map_err(|_| DebuggerError::Unsupported)?;
        for i in 0..width as u32 {
            core.poke_work_ram((addr + i) as usize, (value >> (8 * i)) as u8);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use nerust_core_traits::{CoreConfig, debugger::Debugger as _};
    use nerust_input_traits::{
        Controller, ControllerCollection, EmuInput, OpenBusReadResult, Port,
    };
    use nerust_render_traits::PixelFormat;

    use super::*;
    use crate::{
        debugger::{NesDebugger, SPACE_WORK_RAM},
        input_types::NesInputBuffer,
        nrom_test_data,
    };

    fn test_emu_input() -> EmuInput {
        use nerust_input_traits::InputStateBuffer;
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        EmuInput::new(
            shared,
            Arc::new(AtomicBool::new(false)),
            Box::new(|| Box::<NesInputBuffer>::default()),
        )
    }

    #[derive(Debug)]
    struct MockController;
    impl Controller for MockController {
        fn read(&mut self, _port: &dyn Port) -> OpenBusReadResult {
            OpenBusReadResult::new(0, 0)
        }
        fn write(&mut self, _port: &dyn Port, _value: u8) {}
    }

    fn test_rom() -> Vec<u8> {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);
        rom
    }

    #[test]
    fn load_and_render_frame() {
        let rom = test_rom();

        // parse_rom should succeed
        let cartridge = crate::rom_parse::parse_rom(&rom).expect("parse_rom should succeed");
        assert_eq!(cartridge.mapper_type(), 0);

        // NesConsoleCore::new should succeed
        let mut core = NesConsoleCore::new(
            cartridge,
            ControllerCollection::new(vec![Box::new(MockController)]),
            test_emu_input(),
        )
        .expect("NesConsoleCore::new should succeed");

        // render_frame should succeed
        let mut fb = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        );
        let mut audio = Vec::new();
        let result = core.render_frame(&mut fb, &mut audio);
        assert!(result.is_ok(), "render_frame should succeed: {:?}", result);
        assert!(!audio.is_empty(), "frame must produce audio");
    }

    #[test]
    fn load_via_trait_method() {
        let rom = test_rom();
        let mut core = NesConsoleCore::new_empty(
            ControllerCollection::new(vec![Box::new(MockController)]),
            test_emu_input(),
        );
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
            audio_sample_rate: None,
        };

        // load should succeed via trait method
        let result = ConsoleCore::load(&mut core, &rom, &config);
        assert!(result.is_ok(), "load should succeed: {:?}", result);

        // render_frame should succeed after load
        let mut fb = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        );
        let mut audio = Vec::new();
        let result = core.render_frame(&mut fb, &mut audio);
        assert!(
            result.is_ok(),
            "render_frame after load should succeed: {:?}",
            result
        );
    }

    #[test]
    fn debugger_wiring_follows_load_state() {
        use nerust_core_traits::ConsoleCore as _;

        // Empty console: no ROM, no observers.
        let mut empty = NesConsoleCore::new_empty(
            ControllerCollection::new(vec![Box::new(MockController)]),
            test_emu_input(),
        );
        assert!(empty.debugger().is_none());
        assert!(empty.debug_control().is_none());

        // Loaded console: both observers available.
        let rom = test_rom();
        let cartridge = crate::rom_parse::parse_rom(&rom).expect("parse");
        let mut loaded = NesConsoleCore::new(
            cartridge,
            ControllerCollection::new(vec![Box::new(MockController)]),
            test_emu_input(),
        )
        .expect("console");
        {
            let debugger = loaded.debugger().expect("debugger");
            // SPIKE (iteration 6): PRG ROM space added, table has 4 entries.
            assert_eq!(debugger.spaces().len(), 4);
            assert!(
                debugger
                    .read(crate::debugger::SPACE_WORK_RAM, 0, 1)
                    .is_some()
            );
        }
        assert!(loaded.debug_control().is_some());
    }

    #[test]
    fn nes_write_read_roundtrip_is_little_endian() {
        let mut console = NesConsoleCore::new(
            nrom_test_data(),
            ControllerCollection::new(vec![]),
            test_emu_input(),
        )
        .expect("console");
        {
            let mut control = NesDebugControl::new(&mut console);
            control
                .write_memory(SPACE_WORK_RAM, 0x0100, 2, 0xBEEF)
                .expect("WRAM write");
        }
        let core = console.core_ref().expect("loaded");
        let debugger = NesDebugger::new(core);
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0100, 2), Some(0xBEEF));
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0100, 1), Some(0xEF));
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0101, 1), Some(0xBE));
    }

    #[test]
    fn nes_control_can_be_boxed_and_rejects_in_order() {
        let mut console = NesConsoleCore::new(
            nrom_test_data(),
            ControllerCollection::new(vec![]),
            test_emu_input(),
        )
        .expect("console");
        let mut boxed: Box<dyn DebugControl + '_> = Box::new(NesDebugControl::new(&mut console));
        // BadWidth first: width 0 with unknown SpaceId is still BadWidth.
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0x0100, 0, 0),
            Err(DebuggerError::BadWidth(0))
        );
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0x0100, 1, 0),
            Err(DebuggerError::UnknownSpace(SpaceId(9)))
        );
        assert_eq!(
            boxed.write_memory(SPACE_WORK_RAM, 0x2000, 1, 0),
            Err(DebuggerError::UnmappedAddress {
                space: SPACE_WORK_RAM,
                addr: 0x2000
            })
        );
        assert_eq!(
            boxed.write_memory(SPACE_WORK_RAM, 0x1FFF, 2, 0),
            Err(DebuggerError::UnmappedAddress {
                space: SPACE_WORK_RAM,
                addr: 0x1FFF
            })
        );
        // Step has no cycle-accounted entry point yet: explicit, not silent.
        assert_eq!(
            boxed.step(StepUnit::Instruction),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Instruction))
        );
        // Frame stepping reuses the render path and reports real cycles.
        let cycles = boxed.step(StepUnit::Frame).expect("frame step");
        assert!(cycles > 0);
    }
}
