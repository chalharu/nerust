use std::{sync::Arc, time::SystemTime};

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind,
    audio::StereoSample,
    debugger::{DebugControl, DebuggerError, SpaceAccess, SpaceId, SpaceTable, StepUnit},
    identity::SystemIdentity,
    peripheral::{
        AccelerometerInputPort, RumbleOutputPort, RumbleState, accelerometer_channel,
        rumble_channel,
    },
};
use nerust_input_traits::EmuInput;
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{
    cartridge_descriptor::detect_cartridge, core_options::GbcCoreOptions,
    debugger::GBC_SPACE_TABLE, input_types::GbcInputBuffer, persistence,
    rom_identity::GbcRomIdentity, system::GbcSystem,
};

#[derive(Debug, thiserror::Error)]
enum GbcCoreError {
    #[error("invalid or unsupported Game Boy ROM")]
    InvalidRom,
    #[error("GBC input buffer has the wrong concrete type")]
    InvalidInputBuffer,
}

struct LoadedGbc {
    system: GbcSystem,
    rom: Arc<[u8]>,
    identity: GbcRomIdentity,
    options: GbcCoreOptions,
}

pub struct GbcConsoleCore {
    loaded: Option<LoadedGbc>,
    emu_input: EmuInput,
    paused: bool,
    accelerometer: AccelerometerInputPort,
    rumble: RumbleOutputPort,
    /// Device sample rate for the APU resampler, stamped from
    /// `CoreConfig::audio_sample_rate` at load (data only; the backend
    /// lives in the session layer).
    sample_rate: u32,
}

impl GbcConsoleCore {
    pub fn new_empty(emu_input: EmuInput) -> Self {
        let (_, accelerometer) = accelerometer_channel();
        let (_, rumble) = rumble_channel();
        Self::with_peripherals(emu_input, accelerometer, rumble)
    }

    pub fn with_peripherals(
        emu_input: EmuInput,
        accelerometer: AccelerometerInputPort,
        rumble: RumbleOutputPort,
    ) -> Self {
        Self {
            loaded: None,
            emu_input,
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

    /// Advance one frame, reporting the T-cycles the bus counted.
    ///
    /// Same path as the [`ConsoleCore::render_frame`] implementation;
    /// the count that path discards is returned here for debugger
    /// stepping. One loop iteration is one T-cycle by construction.
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
        self.emu_input.take();
        let input = self
            .emu_input
            .read_buf
            .downcast_ref::<GbcInputBuffer>()
            .ok_or_else(|| CoreError::Core(Box::new(GbcCoreError::InvalidInputBuffer)))?
            .0;
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

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let loaded = self.loaded_ref()?;
        persistence::export_machine_state(
            &loaded.system,
            loaded.identity,
            loaded.options,
            SystemTime::now(),
        )
        .map_err(|error| CoreError::Core(Box::new(error)))
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
        .map_err(|error| CoreError::Core(Box::new(error)))?;
        self.loaded_mut()?.system = candidate.system;
        self.sync_rumble();
        Ok(())
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let loaded = self.loaded_ref()?;
        persistence::export_mapper_save(&loaded.system, loaded.identity, SystemTime::now())
            .map_err(|error| CoreError::Core(Box::new(error)))
    }

    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let loaded = self.loaded_mut()?;
        persistence::import_mapper_save(&mut loaded.system, data, loaded.identity)
            .map_err(|error| CoreError::Core(Box::new(error)))?;
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

/// GBC execution control.
///
/// Holds the console: frame stepping reuses the exact `render_frame`
/// path and reports the T-cycles it counted. The scratch buffer is
/// display-irrelevant; stepped frames are not published to any shared
/// framebuffer here. All spaces are read-only in phase 1, so every
/// in-range write is refused as `ReadOnlySpace`.
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
            StepUnit::Instruction => Err(DebuggerError::UnsupportedStepUnit(unit)),
        }
    }

    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        _value: u64,
    ) -> Result<(), DebuggerError> {
        // Validation order matters: bad width first, so "width 0 with
        // unknown SpaceId" does not mask as UnknownSpace.
        if !SpaceTable::width_is_valid(width) {
            return Err(DebuggerError::BadWidth(width));
        }
        let info = GBC_SPACE_TABLE
            .get(space)
            .ok_or(DebuggerError::UnknownSpace(space))?;
        if !GBC_SPACE_TABLE.covers(space, addr, width) {
            return Err(DebuggerError::UnmappedAddress { space, addr });
        }
        if info.access == SpaceAccess::ReadOnly {
            return Err(DebuggerError::ReadOnlySpace(space));
        }
        Err(DebuggerError::ReadOnlySpace(space))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use nerust_core_traits::{
        CoreOptions,
        peripheral::{AccelerationSample, RumbleState, accelerometer_channel, rumble_channel},
    };
    use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

    use super::*;
    use crate::debugger::SPACE_MEMORY;

    #[derive(Debug, Clone)]
    struct OtherOptions;

    impl CoreOptions for OtherOptions {}

    #[derive(Debug)]
    struct OtherInput;

    impl InputStateBuffer for OtherInput {
        fn set(&mut self, field: usize, _value: InputValue) -> Result<(), BufferError> {
            Err(BufferError::FieldNotFound { field })
        }

        fn clear(&mut self) {}

        fn copy_state(&mut self, _other: &dyn InputStateBuffer) {}
    }

    fn input() -> EmuInput {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<GbcInputBuffer>::default()));
        EmuInput::new(
            shared,
            Arc::new(AtomicBool::new(false)),
            Box::new(|| Box::<GbcInputBuffer>::default()),
        )
    }

    fn wrong_input() -> EmuInput {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::new(OtherInput)));
        EmuInput::new(
            shared,
            Arc::new(AtomicBool::new(false)),
            Box::new(|| Box::new(OtherInput)),
        )
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
        assert!(target.load_state(&state).is_err());
        assert_eq!(target.identity().unwrap(), identity_before);
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
    fn rejects_invalid_rom_options_and_input_type() {
        let mut core = GbcConsoleCore::new_empty(input());
        assert!(matches!(
            core.load(&[], &config()),
            Err(CoreError::RomParse(_))
        ));
        assert!(matches!(
            core.load(&rom(), &config_with_options(OtherOptions)),
            Err(CoreError::InvalidCoreOptions)
        ));

        let mut core = GbcConsoleCore::new_empty(wrong_input());
        core.load(&rom(), &config()).unwrap();
        let mut frame = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        let mut audio = Vec::new();
        assert!(matches!(
            core.render_frame(&mut frame, &mut audio),
            Err(CoreError::Core(_))
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
        // Phase 1 is read-only: even WRAM refuses.
        assert_eq!(
            boxed.write_memory(SPACE_MEMORY, 0xC000, 1, 0),
            Err(DebuggerError::ReadOnlySpace(SPACE_MEMORY))
        );
        // No instruction stepping yet: explicit, not silent.
        assert_eq!(
            boxed.step(StepUnit::Instruction),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Instruction))
        );
        // Frame stepping reuses the render path and reports real cycles.
        let cycles = boxed.step(StepUnit::Frame).expect("frame step");
        assert!(cycles > 0);
        assert!(cycles <= 70_224);
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
}
