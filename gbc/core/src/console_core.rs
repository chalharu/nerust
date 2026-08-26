use std::{sync::Arc, time::SystemTime};

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind, audio::AudioBackend,
    identity::SystemIdentity,
};
use nerust_input_traits::EmuInput;
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{
    core_options::{GbcCoreOptions, RtcSyncPolicy},
    input_types::GbcInputBuffer,
    persistence,
    rom_identity::GbcRomIdentity,
    system::GbcSystem,
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
    audio: Box<dyn AudioBackend>,
    emu_input: EmuInput,
    paused: bool,
}

impl GbcConsoleCore {
    pub fn new_empty(audio: Box<dyn AudioBackend>, emu_input: EmuInput) -> Self {
        Self {
            loaded: None,
            audio,
            emu_input,
            paused: false,
        }
    }

    fn create_loaded(
        rom: &[u8],
        options: GbcCoreOptions,
        sample_rate: u32,
    ) -> Result<LoadedGbc, CoreError> {
        let identity = GbcRomIdentity::from_rom(rom)
            .ok_or_else(|| CoreError::RomParse(Box::new(GbcCoreError::InvalidRom)))?;
        if identity.rom_len < identity.declared_rom_len {
            return Err(CoreError::RomParse(Box::new(GbcCoreError::InvalidRom)));
        }
        let mut system = GbcSystem::from_rom(options.hardware_model, rom.to_vec())
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
}

impl ConsoleCore for GbcConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![PixelFormat::Rgba],
            video_signal: VideoSignalKind::Lcd,
        }
    }

    fn render_frame(&mut self, frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        self.emu_input.take();
        let input = self
            .emu_input
            .read_buf
            .downcast_ref::<GbcInputBuffer>()
            .ok_or_else(|| CoreError::Core(Box::new(GbcCoreError::InvalidInputBuffer)))?
            .0;
        let loaded = self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)?;
        loaded.system.bus.set_joypad(input);
        // LCD-off games do not produce a PPU frame event; cap one frontend
        // frame to the hardware frame duration so the emulation thread stays live.
        for _ in 0..70_224 {
            if loaded.system.bus.step_tcycle(&mut loaded.system.cpu) {
                break;
            }
        }
        for sample in loaded.system.bus.flush_audio() {
            self.audio.push(sample);
        }
        if frame_slot.format() != &PixelFormat::Rgba {
            frame_slot.set_format(PixelFormat::Rgba);
        }
        frame_slot.resize(160, 144);
        loaded.system.bus.render_frame(frame_slot);
        Ok(())
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
        let loaded = Self::create_loaded(rom, options, self.audio.sample_rate())?;
        self.loaded = Some(loaded);
        self.paused = false;
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = None;
        self.paused = false;
    }

    fn reset(&mut self) {
        let sample_rate = self.audio.sample_rate();
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
    }

    fn set_volume(&mut self, volume: f32) {
        self.audio.set_volume(volume);
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let loaded = self.loaded_ref()?;
        persistence::export_machine_state(&loaded.system, loaded.identity, loaded.options)
            .map_err(|error| CoreError::Core(Box::new(error)))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let sample_rate = self.audio.sample_rate();
        let loaded = self.loaded_ref()?;
        let mut candidate = Self::create_loaded(&loaded.rom, loaded.options, sample_rate)?;
        persistence::import_machine_state(
            &mut candidate.system,
            data,
            loaded.identity,
            loaded.options,
        )
        .map_err(|error| CoreError::Core(Box::new(error)))?;
        self.loaded_mut()?.system = candidate.system;
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
        if loaded.options.rtc_sync == RtcSyncPolicy::SystemTime {
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
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use nerust_core_traits::CoreOptions;
    use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

    use super::*;

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

    #[derive(Debug, Default)]
    struct AudioState {
        volume: f32,
        samples: usize,
    }

    struct TestAudio(Arc<Mutex<AudioState>>);

    impl AudioBackend for TestAudio {
        fn start(&mut self) {}

        fn pause(&mut self) {}

        fn push(&mut self, _sample: f32) {
            self.0.lock().unwrap().samples += 1;
        }

        fn set_volume(&mut self, volume: f32) {
            self.0.lock().unwrap().volume = volume;
        }
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
        rom
    }

    fn distinct_rom() -> Vec<u8> {
        let mut value = rom();
        value[0x2000] = 1;
        value
    }

    fn config() -> CoreConfig {
        CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
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
        let mut core =
            GbcConsoleCore::new_empty(Box::new(nerust_core_traits::audio::NullAudio), input());
        core.load(&rom(), &config()).unwrap();
        let mut frame = FrameBuffer::with_capacity(
            160,
            144,
            PixelFormat::PaletteIndex {
                palette: vec![0; 4].into_boxed_slice(),
            },
        );
        core.render_frame(&mut frame).unwrap();
        assert_eq!((frame.width(), frame.height()), (160, 144));
        assert_eq!(frame.format(), &PixelFormat::Rgba);

        let state = core.save_state().unwrap();
        core.load_state(&state).unwrap();
        assert!(!core.identity().unwrap().identity_bytes.is_empty());
    }

    #[test]
    fn empty_core_reports_no_rom() {
        let core =
            GbcConsoleCore::new_empty(Box::new(nerust_core_traits::audio::NullAudio), input());
        assert!(matches!(core.save_state(), Err(CoreError::NoRomLoaded)));
    }

    #[test]
    fn rejects_machine_state_from_another_rom() {
        let mut source =
            GbcConsoleCore::new_empty(Box::new(nerust_core_traits::audio::NullAudio), input());
        source.load(&rom(), &config()).unwrap();
        let state = source.save_state().unwrap();

        let mut target =
            GbcConsoleCore::new_empty(Box::new(nerust_core_traits::audio::NullAudio), input());
        target.load(&distinct_rom(), &config()).unwrap();
        let identity_before = target.identity().unwrap();
        assert!(target.load_state(&state).is_err());
        assert_eq!(target.identity().unwrap(), identity_before);
    }

    #[test]
    fn lifecycle_capabilities_and_volume_delegate() {
        let audio_state = Arc::new(Mutex::new(AudioState::default()));
        let mut core =
            GbcConsoleCore::new_empty(Box::new(TestAudio(Arc::clone(&audio_state))), input());
        let capabilities = core.capabilities();
        assert_eq!(capabilities.video_signal, VideoSignalKind::Lcd);
        assert_eq!(capabilities.output_formats, vec![PixelFormat::Rgba]);

        core.set_volume(0.25);
        assert_eq!(audio_state.lock().unwrap().volume, 0.25);
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
        let mut core =
            GbcConsoleCore::new_empty(Box::new(nerust_core_traits::audio::NullAudio), input());
        assert!(matches!(
            core.load(&[], &config()),
            Err(CoreError::RomParse(_))
        ));
        assert!(matches!(
            core.load(&rom(), &config_with_options(OtherOptions)),
            Err(CoreError::InvalidCoreOptions)
        ));

        let mut core = GbcConsoleCore::new_empty(
            Box::new(nerust_core_traits::audio::NullAudio),
            wrong_input(),
        );
        core.load(&rom(), &config()).unwrap();
        let mut frame = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        assert!(matches!(
            core.render_frame(&mut frame),
            Err(CoreError::Core(_))
        ));
    }

    #[test]
    fn non_battery_rom_has_no_mapper_save() {
        let mut core =
            GbcConsoleCore::new_empty(Box::new(nerust_core_traits::audio::NullAudio), input());
        core.load(&rom(), &config()).unwrap();
        assert!(core.mapper_save().unwrap().is_none());
    }
}
