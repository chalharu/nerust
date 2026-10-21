use std::{sync::Arc, time::SystemTime};

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind,
    audio::StereoSample,
    debugger::{DebugControl, DebuggerError, SpaceId, StepUnit},
    identity::SystemIdentity,
    peripheral::{
        AccelerometerInputPort, RumbleOutputPort, RumbleState, accelerometer_channel,
        rumble_channel,
    },
};
use nerust_input_traits::InputReceiver;
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{
    cartridge_descriptor::detect_cartridge, core_options::GbcCoreOptions, cpu_core::Phase,
    debugger::GBC_SPACE_TABLE, input_types::GbcInputBuffer, persistence,
    rom_identity::GbcRomIdentity, system::GbcSystem,
};

#[derive(Debug, thiserror::Error)]
enum GbcCoreError {
    #[error("invalid or unsupported Game Boy ROM")]
    InvalidRom,
}

struct LoadedGbc {
    system: GbcSystem,
    rom: Arc<[u8]>,
    identity: GbcRomIdentity,
    options: GbcCoreOptions,
}

pub struct GbcConsoleCore {
    loaded: Option<LoadedGbc>,
    emu_input: InputReceiver<GbcInputBuffer>,
    paused: bool,
    accelerometer: AccelerometerInputPort,
    rumble: RumbleOutputPort,
    /// Device sample rate for the APU resampler, stamped at load.
    /// Data only; the backend lives in the session layer.
    sample_rate: u32,
}

impl GbcConsoleCore {
    pub fn new_empty(emu_input: impl Into<InputReceiver<GbcInputBuffer>>) -> Self {
        let (_, accelerometer) = accelerometer_channel();
        let (_, rumble) = rumble_channel();
        Self::with_peripherals(emu_input, accelerometer, rumble)
    }

    pub fn with_peripherals(
        emu_input: impl Into<InputReceiver<GbcInputBuffer>>,
        accelerometer: AccelerometerInputPort,
        rumble: RumbleOutputPort,
    ) -> Self {
        Self {
            loaded: None,
            emu_input: emu_input.into(),
            paused: false,
            accelerometer,
            rumble,
            sample_rate: 48_000,
        }
    }

    fn current_rumble_state(&self) -> RumbleState {
        if self.paused {
            RumbleState::OFF
        } else {
            self.loaded.as_ref().map_or(RumbleState::OFF, |loaded| {
                loaded.system.bus.cartridge_rumble_state()
            })
        }
    }

    fn sync_rumble(&self) {
        self.rumble.publish(self.current_rumble_state());
    }

    fn create_loaded(
        rom: &[u8],
        options: GbcCoreOptions,
        sample_rate: u32,
    ) -> Result<LoadedGbc, CoreError> {
        let descriptor = detect_cartridge(rom)
            .ok_or_else(|| CoreError::RomParse(Box::new(GbcCoreError::InvalidRom)))?;
        let identity = GbcRomIdentity::from_descriptor(rom, &descriptor)
            .ok_or_else(|| CoreError::RomParse(Box::new(GbcCoreError::InvalidRom)))?;
        if identity.rom_len < identity.declared_rom_len {
            return Err(CoreError::RomParse(Box::new(GbcCoreError::InvalidRom)));
        }
        let mut system =
            GbcSystem::from_descriptor(options.hardware_model, rom.to_vec(), &descriptor)
                .ok_or_else(|| CoreError::RomParse(Box::new(GbcCoreError::InvalidRom)))?;
        system.bus.set_audio_sample_rate(sample_rate);
        Ok(LoadedGbc {
            system,
            rom: Arc::from(rom),
            identity,
            options,
        })
    }

    fn loaded_ref(&self) -> Result<&LoadedGbc, CoreError> {
        self.loaded.as_ref().ok_or(CoreError::NoRomLoaded)
    }

    fn loaded_mut(&mut self) -> Result<&mut LoadedGbc, CoreError> {
        self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)
    }

    /// Advance to the next instruction boundary, reporting consumed T-cycles.
    /// A halted CPU waits bounded; STOP with no wake source returns zero.
    pub(crate) fn step_instruction(&mut self) -> Result<u64, CoreError> {
        let loaded = self.loaded_mut()?;
        if loaded.system.bus.is_stopped() {
            return Ok(0);
        }
        let mut elapsed = 0u64;
        loop {
            loaded.system.bus.step_tcycle(&mut loaded.system.cpu);
            elapsed += 1;
            if elapsed >= 1_000_000 {
                break;
            }
            if loaded.system.bus.is_halted_or_stopped() {
                continue;
            }
            if matches!(loaded.system.cpu.phase(), Phase::FetchOpcode) {
                break;
            }
        }
        Ok(elapsed)
    }

    /// Advance one frame, reporting the T-cycles the bus counted.
    /// Same path as [`ConsoleCore::render_frame`]; one loop iteration is one T-cycle.
    pub(crate) fn render_frame_cycles(
        &mut self,
        frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<StereoSample>,
    ) -> Result<u64, CoreError> {
        self.render_frame_inner(frame_slot, audio_out)
    }

    fn render_frame_inner(
        &mut self,
        frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<StereoSample>,
    ) -> Result<u64, CoreError> {
        let input = self.emu_input.receive().0;
        let loaded = self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)?;
        loaded
            .system
            .bus
            .set_cartridge_acceleration(self.accelerometer.latest());
        loaded.system.bus.set_joypad(input);
        // LCD-off games do not produce a PPU frame event; cap one frontend
        // frame to the hardware frame duration so the emulation thread stays live.
        let mut cycles = 0u64;
        for _ in 0..70_224 {
            cycles += 1;
            if loaded.system.bus.step_tcycle(&mut loaded.system.cpu) {
                break;
            }
        }
        // Nominal-rate audio production only: the caller (session layer)
        // owns transport through the rate-control filter to the backend.
        audio_out.clear();
        audio_out.extend(loaded.system.bus.flush_audio());
        if frame_slot.format() != &PixelFormat::Rgba {
            frame_slot.set_format(PixelFormat::Rgba);
        }
        frame_slot.resize(160, 144);
        loaded.system.bus.render_frame(frame_slot);
        self.sync_rumble();
        Ok(cycles)
    }
}

impl ConsoleCore for GbcConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![PixelFormat::Rgba],
            video_signal: VideoSignalKind::Lcd,
        }
    }

    fn render_frame(
        &mut self,
        frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<StereoSample>,
    ) -> Result<(), CoreError> {
        self.render_frame_inner(frame_slot, audio_out).map(|_| ())
    }

    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError> {
        let options = if let Some(options) = &config.core_options {
            *options
                .clone()
                .downcast::<GbcCoreOptions>()
                .map_err(|_| CoreError::InvalidCoreOptions)?
        } else {
            GbcCoreOptions::default()
        };
        let loaded = Self::create_loaded(rom, options, config.audio_sample_rate.unwrap_or(48_000))?;
        self.sample_rate = config.audio_sample_rate.unwrap_or(48_000);
        self.accelerometer
            .set_requested(loaded.identity.cartridge_type == 0x22);
        self.loaded = Some(loaded);
        self.paused = false;
        self.sync_rumble();
        Ok(())
    }

    fn unload(&mut self) {
        self.accelerometer.set_requested(false);
        self.loaded = None;
        self.paused = false;
        self.sync_rumble();
    }

    fn reset(&mut self) {
        let sample_rate = self.sample_rate;
        let Some(current) = self.loaded.as_mut() else {
            return;
        };
        let Ok(mut reset) = Self::create_loaded(&current.rom, current.options, sample_rate) else {
            log::error!("failed to rebuild validated GBC ROM during reset");
            return;
        };
        let mut cartridge = current.system.bus.take_cartridge();
        cartridge.reset_runtime();
        reset.system.bus.set_cartridge(cartridge);
        current.system = reset.system;
        self.sync_rumble();
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.sync_rumble();
    }

    fn set_audio_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        // Re-basis the APU resampler + filters (same call as load);
        // unloaded cores just carry the stamp for the next load.
        if let Ok(loaded) = self.loaded_mut() {
            loaded.system.bus.set_audio_sample_rate(sample_rate);
        }
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let loaded = self.loaded_ref()?;
        persistence::export_machine_state(
            &loaded.system,
            loaded.identity,
            loaded.options,
            SystemTime::now(),
        )
        .map_err(CoreError::from)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let sample_rate = self.sample_rate;
        let loaded = self.loaded_ref()?;
        let mut candidate = Self::create_loaded(&loaded.rom, loaded.options, sample_rate)?;
        persistence::import_machine_state(
            &mut candidate.system,
            data,
            loaded.identity,
            loaded.options,
            SystemTime::now(),
        )
        .map_err(CoreError::from)?;
        self.loaded_mut()?.system = candidate.system;
        self.sync_rumble();
        Ok(())
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let loaded = self.loaded_ref()?;
        persistence::export_mapper_save(&loaded.system, loaded.identity, SystemTime::now())
            .map_err(CoreError::from)
    }

    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let loaded = self.loaded_mut()?;
        persistence::import_mapper_save(&mut loaded.system, data, loaded.identity)
            .map_err(CoreError::from)?;
        if loaded.options.rtc_sync.syncs_save_data() {
            loaded.system.bus.sync_cartridge_rtc(SystemTime::now());
        }
        Ok(())
    }

    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.loaded_ref()?
            .identity
            .into_system_identity()
            .map_err(|error| CoreError::Core(Box::new(error)))
    }

    fn debugger(&self) -> Option<Box<dyn nerust_core_traits::debugger::Debugger + '_>> {
        self.loaded
            .as_ref()
            .map(|loaded| Box::new(crate::debugger::GbcDebugger::new(&loaded.system)) as _)
    }

    fn debug_control(
        &mut self,
    ) -> Option<Box<dyn nerust_core_traits::debugger::DebugControl + '_>> {
        self.loaded.as_mut()?;
        Some(Box::new(GbcDebugControl::new(self)) as _)
    }

    fn output_channels(&self) -> Vec<String> {
        vec!["serial".to_string()]
    }

    fn take_channel_bytes(&mut self, channel: &str) -> Vec<u8> {
        if channel != "serial" {
            return Vec::new();
        }
        self.loaded
            .as_mut()
            .map(|loaded| loaded.system.bus.take_serial_output())
            .unwrap_or_default()
    }
}

/// GBC execution control and memory editing.
/// Holds the console: frame stepping reuses the exact `render_frame` path.
pub struct GbcDebugControl<'a> {
    console: &'a mut GbcConsoleCore,
    frame_slot: FrameBuffer,
    audio_sink: Vec<StereoSample>,
}

impl<'a> GbcDebugControl<'a> {
    pub fn new(console: &'a mut GbcConsoleCore) -> Self {
        let mut frame_slot = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        frame_slot.resize(160, 144);
        Self {
            console,
            frame_slot,
            audio_sink: Vec::new(),
        }
    }
}

impl DebugControl for GbcDebugControl<'_> {
    fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError> {
        match unit {
            StepUnit::Frame => self
                .console
                .render_frame_cycles(&mut self.frame_slot, &mut self.audio_sink)
                .map_err(|_| DebuggerError::Unsupported),
            StepUnit::Instruction => self
                .console
                .step_instruction()
                .map_err(|_| DebuggerError::Unsupported),
        }
    }

    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        value: u64,
    ) -> Result<(), DebuggerError> {
        // One table-owned gate in fixed order: width, space, address, ROM refusal.
        // The cast below is safe: check_write pins addr + width - 1.
        GBC_SPACE_TABLE.check_write(space, addr, width)?;
        let loaded = self
            .console
            .loaded
            .as_mut()
            .ok_or(DebuggerError::Unsupported)?;
        for i in 0..width as u32 {
            loaded
                .system
                .bus
                .debug_write((addr + i) as u16, (value >> (8 * i)) as u8);
        }
        Ok(())
    }

    fn write_register(&mut self, name: &str, value: u64) -> Result<(), DebuggerError> {
        let loaded = self
            .console
            .loaded
            .as_mut()
            .ok_or(DebuggerError::Unsupported)?;
        // Values mask to the register width; unknown names refuse.
        // A mid-instruction poke applies immediately, normalized at the next boundary.
        let regs = loaded.system.cpu.registers_mut();
        match name {
            "a" => regs.set_a(value as u8),
            "b" => regs.set_b(value as u8),
            "c" => regs.set_c(value as u8),
            "d" => regs.set_d(value as u8),
            "e" => regs.set_e(value as u8),
            "f" => regs.set_f(value as u8),
            "h" => regs.set_h(value as u8),
            "l" => regs.set_l(value as u8),
            "pc" => regs.set_pc(value as u16),
            "sp" => regs.set_sp(value as u16),
            _ => return Err(DebuggerError::Unsupported),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nerust_core_traits::{
        CoreOptions,
        peripheral::{AccelerationSample, RumbleState, accelerometer_channel, rumble_channel},
    };

    use super::*;
    use crate::debugger::SPACE_MEMORY;

    #[derive(Debug, Clone)]
    struct OtherOptions;

    impl CoreOptions for OtherOptions {}

    fn input() -> InputReceiver<GbcInputBuffer> {
        GbcInputBuffer::default().into()
    }

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0100] = 0x18; // JR -2
        rom[0x0101] = 0xFE;
        rom[0x0143] = 0x80;
        rom[0x0147] = 0;
        rom[0x0148] = 0;
        rom[0x0149] = 0;
        crate::cartridge_header::finalize_test_rom(&mut rom);
        rom
    }

    fn distinct_rom() -> Vec<u8> {
        let mut value = rom();
        value[0x2000] = 1;
        value
    }

    fn mapper_rom(cartridge_type: u8) -> Vec<u8> {
        let mut value = rom();
        value[0x0147] = cartridge_type;
        crate::cartridge_header::finalize_test_rom(&mut value);
        value
    }

    fn config() -> CoreConfig {
        CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
            audio_sample_rate: None,
        }
    }

    fn config_with_options(options: impl CoreOptions + 'static) -> CoreConfig {
        CoreConfig {
            core_options: Some(Box::new(options)),
            ..config()
        }
    }

    #[test]
    fn set_audio_sample_rate_rebasis_without_reconstruction() {
        use nerust_core_traits::ConsoleCore;

        let mut core = GbcConsoleCore::new_empty(input());
        // Unloaded: stamps only, never panics.
        core.set_audio_sample_rate(44_100);
        assert_eq!(core.sample_rate, 44_100);

        core.load(&rom(), &config()).unwrap();
        let mut frame = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        let mut audio = Vec::new();
        core.render_frame(&mut frame, &mut audio).unwrap();

        // Loaded: APU re-basis, machine state intact, frames keep flowing.
        core.set_audio_sample_rate(22_050);
        assert_eq!(core.sample_rate, 22_050);
        core.render_frame(&mut frame, &mut audio).unwrap();
        assert!(
            audio
                .iter()
                .all(|s| s.left.is_finite() && s.right.is_finite())
        );
        let state = core.save_state().unwrap();
        core.load_state(&state).unwrap();
    }

    #[test]
    fn load_render_and_state_round_trip() {
        let mut core = GbcConsoleCore::new_empty(input());
        core.load(&rom(), &config()).unwrap();
        let mut frame = FrameBuffer::with_capacity(
            160,
            144,
            PixelFormat::PaletteIndex {
                palette: vec![0; 4].into_boxed_slice(),
            },
        );
        core.render_frame(&mut frame, &mut Vec::new()).unwrap();
        assert_eq!((frame.width(), frame.height()), (160, 144));
        assert_eq!(frame.format(), &PixelFormat::Rgba);

        let state = core.save_state().unwrap();
        core.load_state(&state).unwrap();
        assert!(!core.identity().unwrap().identity_bytes.is_empty());
    }

    #[test]
    fn render_frame_delivers_finite_stereo_samples() {
        let mut core = GbcConsoleCore::new_empty(input());
        core.load(&rom(), &config()).unwrap();
        let mut frame = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        let mut audio = Vec::new();
        core.render_frame(&mut frame, &mut audio).unwrap();

        assert!(!audio.is_empty());
        assert!(
            audio
                .iter()
                .all(|sample| sample.left.is_finite() && sample.right.is_finite())
        );
    }

    #[test]
    fn empty_core_reports_no_rom() {
        let core = GbcConsoleCore::new_empty(input());
        assert!(matches!(core.save_state(), Err(CoreError::NoRomLoaded)));
    }

    #[test]
    fn rejects_machine_state_from_another_rom() {
        let mut source = GbcConsoleCore::new_empty(input());
        source.load(&rom(), &config()).unwrap();
        let state = source.save_state().unwrap();

        let mut target = GbcConsoleCore::new_empty(input());
        target.load(&distinct_rom(), &config()).unwrap();
        let identity_before = target.identity().unwrap();
        assert_eq!(
            target.load_state(&state).unwrap_err().fault(),
            nerust_core_traits::CoreFault::Incompatible
        );
        assert_eq!(target.identity().unwrap(), identity_before);
    }

    #[test]
    fn persistence_faults_distinguish_options_schema_and_decode() {
        use crate::{persistence_error::GbcPersistenceError, system::HardwareModel};
        use nerust_core_traits::CoreFault;
        #[derive(serde::Serialize)]
        struct Header {
            schema_version: u32,
        }
        let mut core = GbcConsoleCore::new_empty(input());
        core.load(&rom(), &config()).unwrap();
        let saved = core.save_state().unwrap();
        let options = GbcCoreOptions {
            hardware_model: HardwareModel::Dmg,
            ..GbcCoreOptions::default()
        };
        core.load(&rom(), &config_with_options(options)).unwrap();
        let error = core.load_state(&saved).unwrap_err();
        assert_eq!(error.fault(), CoreFault::Incompatible);
        let CoreError::Persistence { source, .. } = error else {
            panic!("untyped options error")
        };
        assert!(matches!(
            source.downcast_ref::<GbcPersistenceError>(),
            Some(GbcPersistenceError::OptionsMismatch)
        ));
        for (version, fault) in [(3, CoreFault::Incompatible), (2, CoreFault::Corrupt)] {
            let bytes = rmp_serde::to_vec_named(&Header {
                schema_version: version,
            })
            .unwrap();
            assert_eq!(core.load_state(&bytes).unwrap_err().fault(), fault);
            assert_eq!(core.import_mapper_save(&bytes).unwrap_err().fault(), fault);
        }
        let error = core.load_state(&saved[..saved.len() / 2]).unwrap_err();
        assert_eq!(error.fault(), CoreFault::Corrupt);
        let CoreError::Persistence { source, .. } = error else {
            panic!("untyped decode error")
        };
        assert!(matches!(
            source.downcast_ref::<GbcPersistenceError>(),
            Some(GbcPersistenceError::Decode(_))
        ));
        core.load(&rom(), &config()).unwrap();
        core.load_state(&saved).unwrap();
    }

    #[test]
    fn lifecycle_capabilities() {
        let mut core = GbcConsoleCore::new_empty(input());
        let capabilities = core.capabilities();
        assert_eq!(capabilities.video_signal, VideoSignalKind::Lcd);
        assert_eq!(capabilities.output_formats, vec![PixelFormat::Rgba]);

        assert!(!core.paused());
        core.set_paused(true);
        assert!(core.paused());
        core.reset();

        core.load(&rom(), &config()).unwrap();
        core.set_paused(true);
        core.reset();
        assert!(core.identity().is_ok());
        core.unload();
        assert!(!core.paused());
        assert!(matches!(core.identity(), Err(CoreError::NoRomLoaded)));
    }

    #[test]
    fn rejects_invalid_rom_and_options() {
        let mut core = GbcConsoleCore::new_empty(input());
        assert!(matches!(
            core.load(&[], &config()),
            Err(CoreError::RomParse(_))
        ));
        assert!(matches!(
            core.load(&rom(), &config_with_options(OtherOptions)),
            Err(CoreError::InvalidCoreOptions)
        ));
    }

    #[test]
    fn non_battery_rom_has_no_mapper_save() {
        let mut core = GbcConsoleCore::new_empty(input());
        core.load(&rom(), &config()).unwrap();
        assert!(core.mapper_save().unwrap().is_none());
    }

    #[test]
    fn mbc7_requests_and_latches_live_acceleration() {
        let (accelerometer_handle, accelerometer_port) = accelerometer_channel();
        let (_, rumble_port) = rumble_channel();
        let mut core = GbcConsoleCore::with_peripherals(input(), accelerometer_port, rumble_port);
        core.load(&mapper_rom(0x22), &config()).unwrap();
        assert!(accelerometer_handle.demand().requested);
        accelerometer_handle.publish(AccelerationSample::new(1.0, -1.0));
        let mut frame = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        core.render_frame(&mut frame, &mut Vec::new()).unwrap();

        let bus = &mut core.loaded.as_mut().unwrap().system.bus;
        bus.write(0, 0x0A);
        bus.write(0x4000, 0x40);
        bus.write(0xA000, 0x55);
        bus.write(0xA010, 0xAA);
        assert_eq!(
            u16::from_le_bytes([bus.read(0xA020), bus.read(0xA030)]),
            0x8240
        );
        assert_eq!(
            u16::from_le_bytes([bus.read(0xA040), bus.read(0xA050)]),
            0x8160
        );

        core.unload();
        assert!(!accelerometer_handle.demand().requested);
    }

    #[test]
    fn mbc5_rumble_tracks_frame_pause_resume_and_unload() {
        let (_, accelerometer_port) = accelerometer_channel();
        let (rumble_handle, rumble_port) = rumble_channel();
        let mut core = GbcConsoleCore::with_peripherals(input(), accelerometer_port, rumble_port);
        core.load(&mapper_rom(0x1C), &config()).unwrap();
        core.loaded.as_mut().unwrap().system.bus.write(0x4000, 0x08);
        let mut frame = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        core.render_frame(&mut frame, &mut Vec::new()).unwrap();
        assert_eq!(rumble_handle.snapshot().state, RumbleState::FULL);

        core.set_paused(true);
        assert_eq!(rumble_handle.snapshot().state, RumbleState::OFF);
        core.set_paused(false);
        assert_eq!(rumble_handle.snapshot().state, RumbleState::FULL);
        core.unload();
        assert_eq!(rumble_handle.snapshot().state, RumbleState::OFF);
    }

    #[test]
    fn gbc_control_can_be_boxed_and_rejects_in_order() {
        let mut console = GbcConsoleCore::new_empty(input());
        console.load(&rom(), &config()).expect("test ROM loads");
        let mut boxed: Box<dyn DebugControl + '_> = Box::new(GbcDebugControl::new(&mut console));
        // BadWidth first: width 0 with unknown SpaceId is still BadWidth.
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0xC000, 0, 0),
            Err(DebuggerError::BadWidth(0))
        );
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0xC000, 1, 0),
            Err(DebuggerError::UnknownSpace(SpaceId(9)))
        );
        assert_eq!(
            boxed.write_memory(SPACE_MEMORY, 0x10000, 1, 0),
            Err(DebuggerError::UnmappedAddress {
                space: SPACE_MEMORY,
                addr: 0x10000
            })
        );
        // Writes land where hardware is writable; ROM refuses
        // loudly instead of driving MBC commands.
        assert_eq!(boxed.write_memory(SPACE_MEMORY, 0xC000, 1, 0x42), Ok(()));
        assert_eq!(
            boxed.write_memory(SPACE_MEMORY, 0x0100, 1, 0x42),
            Err(DebuggerError::ReadOnlySpace(SPACE_MEMORY))
        );
        // Instruction stepping runs one instruction with real cycles.
        let instr_cycles = boxed.step(StepUnit::Instruction).expect("instr step");
        assert!(instr_cycles > 0);
        assert!(instr_cycles <= 10_000);
        // Frame stepping reuses the render path and reports real cycles.
        let cycles = boxed.step(StepUnit::Frame).expect("frame step");
        assert!(cycles > 0);
        assert!(cycles <= 70_224);
        // Round trip through the debugger read path. The frame step
        // above runs guest code, so re-poke first for determinism.
        assert_eq!(boxed.write_memory(SPACE_MEMORY, 0xC010, 1, 0xA5), Ok(()));
        drop(boxed);
        let debugger = console.debugger().expect("debugger");
        assert_eq!(debugger.read(SPACE_MEMORY, 0xC010, 1), Some(0xA5));
    }

    #[test]
    fn gbc_debugger_wiring_follows_load_state() {
        let mut empty = GbcConsoleCore::new_empty(input());
        assert!(empty.debugger().is_none());
        assert!(empty.debug_control().is_none());

        empty.load(&rom(), &config()).expect("test ROM loads");
        assert!(empty.debugger().is_some());
        assert!(empty.debug_control().is_some());

        empty.unload();
        assert!(empty.debugger().is_none());
        assert!(empty.debug_control().is_none());
    }

    #[test]
    fn gbc_register_write_roundtrips_and_refuses_unknown() {
        let mut console = GbcConsoleCore::new_empty(input());
        console.load(&rom(), &config()).expect("test ROM loads");
        {
            let mut boxed: Box<dyn DebugControl + '_> =
                Box::new(GbcDebugControl::new(&mut console));
            boxed.write_register("a", 0x42).expect("a write");
            boxed.write_register("pc", 0x1234).expect("pc write");
            assert_eq!(
                boxed.write_register("bogus", 0),
                Err(DebuggerError::Unsupported)
            );
        }
        let debugger = console.debugger().expect("debugger");
        let regs: std::collections::HashMap<&str, u64> =
            debugger.registers().iter().copied().collect();
        assert_eq!(regs["a"], 0x42);
        assert_eq!(regs["pc"], 0x1234);
    }

    #[test]
    fn gbc_debugger_truncated_tail_degrades_to_raw_db() {
        let mut console = GbcConsoleCore::new_empty(input());
        console.load(&rom(), &config()).expect("test ROM loads");
        // Park a two-byte prefix at the range end: the missing tail byte
        // must degrade to raw instead of fabricating or panicking.
        {
            let mut boxed: Box<dyn DebugControl + '_> =
                Box::new(GbcDebugControl::new(&mut console));
            boxed
                .write_memory(SPACE_MEMORY, 0xFFFF, 1, 0xCB)
                .expect("range end takes the prefix");
        }
        let debugger = console.debugger().expect("debugger");
        let tail = debugger.disassemble(0xFFFF, 2);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].addr, 0xFFFF);
        assert!(tail[0].text.starts_with(".DB"));
    }
}
