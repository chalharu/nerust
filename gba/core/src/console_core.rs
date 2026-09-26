use std::sync::Arc;

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind,
    audio::{AudioBackend, StereoSample},
    identity::SystemIdentity,
};
use nerust_input_traits::EmuInput;
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{
    core_options::GbaCoreOptions,
    input_types::GbaInputBuffer,
    persistence::{
        export_machine_state, export_mapper_save, import_machine_state, import_mapper_save,
    },
    persistence_error::GbaLoadError,
    rom_identity::GbaRomIdentity,
    system::GbaSystem,
};

struct LoadedGba {
    system: GbaSystem,
    rom: Arc<[u8]>,
    identity: GbaRomIdentity,
    options: GbaCoreOptions,
}

pub struct GbaConsoleCore {
    loaded: Option<LoadedGba>,
    audio: Box<dyn AudioBackend>,
    emu_input: EmuInput,
    paused: bool,
    /// Resample scratch reused every frame: `drain_resampled_into`
    /// fills it instead of allocating a fresh Vec per frame.
    resample_scratch: Vec<StereoSample>,
}

impl GbaConsoleCore {
    pub fn new(audio: Box<dyn AudioBackend>, emu_input: EmuInput) -> Self {
        Self {
            loaded: None,
            audio,
            emu_input,
            paused: false,
            resample_scratch: Vec::new(),
        }
    }

    pub fn new_empty(audio: Box<dyn AudioBackend>, emu_input: EmuInput) -> Self {
        Self::new(audio, emu_input)
    }

    fn create_loaded(rom: &[u8], options: GbaCoreOptions) -> Result<LoadedGba, CoreError> {
        let identity = GbaRomIdentity::from_rom(rom)
            .ok_or_else(|| CoreError::RomParse(Box::new(GbaLoadError::InvalidRom)))?;
        let mut system = GbaSystem::from_test_rom(rom.to_vec())
            .or_else(|| GbaSystem::from_rom(rom.to_vec()))
            .ok_or_else(|| CoreError::RomParse(Box::new(GbaLoadError::InvalidRom)))?;
        if let Some(cart) = system.bus.cartridge_mut() {
            cart.gpio.set_solar_level(options.solar_light_level);
        }
        Ok(LoadedGba {
            system,
            rom: Arc::from(rom),
            identity,
            options,
        })
    }

    fn loaded_ref(&self) -> Result<&LoadedGba, CoreError> {
        self.loaded.as_ref().ok_or(CoreError::NoRomLoaded)
    }
}

impl ConsoleCore for GbaConsoleCore {
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
            .downcast_ref::<GbaInputBuffer>()
            .ok_or_else(|| CoreError::Core(Box::new(GbaLoadError::InvalidInputBuffer)))?
            .0;
        let loaded = self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)?;
        loaded.system.bus.set_keyinput(input);
        // Run one LCD frame (228 lines * 1232 cycles), batched to the
        // frame end. Bit-identical to per-cycle stepping.
        loaded.system.step_batch(280_896);
        // Drain native-grid audio at the device rate, reusing the
        // frame scratch (no per-frame allocation).
        let rate = self.audio.sample_rate();
        loaded
            .system
            .bus
            .apu_mut()
            .drain_resampled_into(rate, &mut self.resample_scratch);
        for sample in self.resample_scratch.iter().copied() {
            self.audio.push(sample);
        }
        if frame_slot.format() != &PixelFormat::Rgba {
            frame_slot.set_format(PixelFormat::Rgba);
        }
        frame_slot.resize(240, 160);
        let fb = loaded.system.bus.frame_buffer();
        // Copy RGBA8888 u32 -> u8 with stride handling (stride is 1024 for 240*4)
        let stride = frame_slot.stride();
        let dst = frame_slot.as_mut();
        for y in 0..160 {
            let src_row = &fb[y * 240..(y + 1) * 240];
            let src_bytes =
                unsafe { std::slice::from_raw_parts(src_row.as_ptr() as *const u8, 240 * 4) };
            let dst_offset = y * stride;
            dst[dst_offset..dst_offset + 240 * 4].copy_from_slice(src_bytes);
        }
        Ok(())
    }

    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError> {
        let options = if let Some(options) = &config.core_options {
            *options
                .clone()
                .downcast::<GbaCoreOptions>()
                .map_err(|_| CoreError::InvalidCoreOptions)?
        } else {
            GbaCoreOptions::default()
        };
        self.loaded = Some(Self::create_loaded(rom, options)?);
        self.paused = false;
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = None;
        self.paused = false;
    }

    fn reset(&mut self) {
        let Some(current) = self.loaded.as_ref() else {
            return;
        };
        // Battery-backed RAM and the GPIO RTC keep running across a reset
        // (like the hardware reset button); everything else rebuilds.
        let reset_state = current
            .system
            .bus
            .cartridge()
            .map(|cart| cart.export_state());
        let Ok(mut fresh) = Self::create_loaded(&current.rom, current.options) else {
            return;
        };
        if let (Some(state), Some(cart)) = (reset_state, fresh.system.bus.cartridge_mut()) {
            if cart.import_state(&state).is_err() {
                // Never swap a reset that drops the battery: keep running.
                return;
            }
            // The solar level follows options, not the transplanted runtime.
            cart.gpio.set_solar_level(current.options.solar_light_level);
        }
        self.loaded = Some(fresh);
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let loaded = self.loaded_ref()?;
        export_machine_state(
            &loaded.system,
            loaded.identity.clone(),
            loaded.options,
            self.audio.sample_rate(),
        )
        .map_err(|e| CoreError::Core(Box::new(e)))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let loaded = self.loaded_ref()?;
        let mut candidate = import_machine_state(
            data,
            &loaded.rom,
            &loaded.identity,
            loaded.options,
            self.audio.sample_rate(),
        )
        .map_err(|e| CoreError::Core(Box::new(e)))?;
        // Solar level lives in options; re-apply it onto the restored GPIO.
        if let Some(cart) = candidate.bus.cartridge_mut() {
            cart.gpio.set_solar_level(loaded.options.solar_light_level);
        }
        let loaded = self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)?;
        loaded.system = candidate;
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) {
        self.audio.set_volume(volume);
    }

    fn restart_audio(&mut self) {
        self.audio.start();
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let loaded = self.loaded_ref()?;
        let Some(cart) = loaded.system.bus.cartridge() else {
            return Ok(None);
        };
        export_mapper_save(cart, loaded.identity.clone()).map_err(|e| CoreError::Core(Box::new(e)))
    }

    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let loaded = self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)?;
        let Some(cart) = loaded.system.bus.cartridge_mut() else {
            return Err(CoreError::Core(Box::new(
                crate::persistence_error::GbaPersistenceError::Cartridge(
                    "no cartridge loaded".to_string(),
                ),
            )));
        };
        import_mapper_save(cart, data, &loaded.identity).map_err(|e| CoreError::Core(Box::new(e)))
    }

    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        let loaded = self.loaded_ref()?;
        loaded
            .identity
            .clone()
            .into_system_identity()
            .map_err(|e| CoreError::Core(Box::new(std::io::Error::other(e))))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use nerust_core_traits::{CoreConfig, audio::NullAudio};
    use nerust_input_traits::{EmuInput, InputStateBuffer};

    use super::*;
    use crate::input_types::GbaInputBuffer;

    fn test_emu_input() -> EmuInput {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<GbaInputBuffer>::default()));
        EmuInput::new(
            shared,
            Arc::new(AtomicBool::new(false)),
            Box::new(|| Box::<GbaInputBuffer>::default()),
        )
    }

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x4000];
        // Minimal GBA header with valid logo/complement
        let logo: [u8; 156] = [
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom[4..160].copy_from_slice(&logo);
        rom[0xB2] = 0x96;
        // fixed and complement
        rom[0xA0..0xBC].copy_from_slice(&[0u8; 28]);
        // complement
        let mut chk: u8 = 0;
        for b in &rom[0xA0..0xBD] {
            chk = chk.wrapping_sub(*b).wrapping_sub(1);
        }
        rom[0xBD] = chk;
        crate::cartridge::header::finalize_test_gba_rom(&mut rom);
        rom
    }

    #[test]
    fn capabilities_are_correct() {
        let core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        let caps = core.capabilities();
        assert_eq!(caps.output_formats.len(), 1);
    }

    #[test]
    fn mapper_save_round_trips_backup_ram() {
        use nerust_core_traits::ConsoleCore;
        fn sram_rom() -> Vec<u8> {
            let mut rom = rom();
            // Backup-ID scan finds SRAM_V (word-aligned) -> Sram backend.
            let tag = b"SRAM_V00";
            rom[0x1000..0x1000 + tag.len()].copy_from_slice(tag);
            rom
        }
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
        };
        let rom_data = sram_rom();
        let mut a = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        a.load(&rom_data, &config).unwrap();
        // No backup chip on the plain header ROM -> no save payload.
        let mut plain = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        plain.load(&rom(), &config).unwrap();
        assert_eq!(plain.mapper_save().unwrap(), None);
        // SRAM chip: write, export, re-import into a fresh core, read back.
        a.loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .write8(0x0E000123, 0x5A);
        let payload = a.mapper_save().unwrap().expect("SRAM save payload");
        let mut b = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        b.load(&rom_data, &config).unwrap();
        b.import_mapper_save(&payload).unwrap();
        assert_eq!(
            b.loaded.as_mut().unwrap().system.bus.read8(0x0E000123),
            0x5A
        );
    }

    #[test]
    fn load_render_and_state_round_trip() {
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(
            &rom(),
            &CoreConfig {
                region: None,
                bios_paths: HashMap::new(),
                controllers: HashMap::new(),
                core_options: None,
            },
        )
        .unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
            240,
            160,
            nerust_render_traits::PixelFormat::Rgba,
        );
        core.render_frame(&mut frame).unwrap();
        assert_eq!((frame.width(), frame.height()), (240, 160));
        let state = core.save_state().unwrap();
        // Advance a frame, then restore: the re-export must match exactly.
        core.render_frame(&mut frame).unwrap();
        core.load_state(&state).unwrap();
        let again = core.save_state().unwrap();
        assert_eq!(state, again);
    }

    #[test]
    fn failed_import_leaves_system_untouched() {
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(
            &rom(),
            &CoreConfig {
                region: None,
                bios_paths: HashMap::new(),
                controllers: HashMap::new(),
                core_options: None,
            },
        )
        .unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
            240,
            160,
            nerust_render_traits::PixelFormat::Rgba,
        );
        core.render_frame(&mut frame).unwrap();
        let before = core.save_state().unwrap();
        // Garbage and truncated payloads must not panic or mutate state.
        assert!(core.load_state(&[0xDE, 0xAD, 0xBE, 0xEF]).is_err());
        assert!(core.load_state(&before[..before.len() / 2]).is_err());
        let mut wrong_version = before.clone();
        // Corrupt the schema version field deep inside the payload is
        // fragile; flipping the first byte breaks the map header instead,
        // which must still decode as an error, not a panic.
        wrong_version[0] ^= 0xFF;
        assert!(core.load_state(&wrong_version).is_err());
        assert_eq!(core.save_state().unwrap(), before);
    }

    #[test]
    fn mapper_save_envelope_rejects_foreign_payloads() {
        fn sram_rom() -> Vec<u8> {
            let mut rom = rom();
            let tag = b"SRAM_V00";
            rom[0x1000..0x1000 + tag.len()].copy_from_slice(tag);
            rom
        }
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
        };
        let rom_data = sram_rom();
        let mut a = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        a.load(&rom_data, &config).unwrap();
        let payload = a.mapper_save().unwrap().expect("SRAM save payload");
        // A foreign ROM (different CRC) must refuse the envelope.
        let mut other = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        let mut rom2 = rom_data.clone();
        rom2[0x200] ^= 1;
        crate::cartridge::header::finalize_test_gba_rom(&mut rom2);
        other.load(&rom2, &config).unwrap();
        assert!(other.import_mapper_save(&payload).is_err());
        // Garbage must not panic or wipe RAM.
        a.loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .write8(0x0E000123, 0x5A);
        assert!(a.import_mapper_save(&[1, 2, 3]).is_err());
        assert_eq!(
            a.loaded.as_mut().unwrap().system.bus.read8(0x0E000123),
            0x5A
        );
    }

    #[test]
    fn reset_preserves_battery_and_gpio_rtc() {
        fn sram_rom() -> Vec<u8> {
            let mut rom = rom();
            let tag = b"SRAM_V00";
            rom[0x1000..0x1000 + tag.len()].copy_from_slice(tag);
            rom
        }
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: Some(Box::new(GbaCoreOptions {
                solar_light_level: 0x20,
            })),
        };
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(&sram_rom(), &config).unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
            240,
            160,
            nerust_render_traits::PixelFormat::Rgba,
        );
        core.render_frame(&mut frame).unwrap();
        // Battery RAM, GPIO attachment and solar level are set pre-reset.
        // The solar level comes from options (the only production source);
        // reset must not clobber the options-applied level.
        core.loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .write8(0x0E000123, 0xA5);
        let cart = core
            .loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .cartridge_mut()
            .unwrap();
        cart.gpio.write(0x080000C8, 2, 1);
        core.reset();
        let loaded = core.loaded.as_mut().unwrap();
        // Runtime restarts from boot (pipeline filled: execute+8).
        assert_eq!(loaded.system.cpu.registers().pc(), 0x08000008);
        // ...while battery RAM, GPIO attachment and solar level survive.
        assert_eq!(loaded.system.bus.read8(0x0E000123), 0xA5);
        let cart = loaded.system.bus.cartridge_mut().unwrap();
        assert!(cart.gpio.is_attached());
        assert_eq!(cart.gpio.solar_level(), 0x20);
    }

    #[test]
    fn save_state_payload_size_is_rewind_input() {
        fn sized_rom(tag: Option<&[u8]>) -> Vec<u8> {
            let mut rom = rom();
            if let Some(tag) = tag {
                rom[0x1000..0x1000 + tag.len()].copy_from_slice(tag);
            }
            rom
        }
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
        };
        for (name, tag) in [
            ("plain", None),
            ("sram", Some(b"SRAM_V00".as_slice())),
            ("flash", Some(b"FLASH_V130".as_slice())),
            ("eeprom", Some(b"EEPROM_V124".as_slice())),
        ] {
            let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
            core.load(&sized_rom(tag), &config).unwrap();
            let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
                240,
                160,
                nerust_render_traits::PixelFormat::Rgba,
            );
            core.render_frame(&mut frame).unwrap();
            let state = core.save_state().unwrap();
            // Recorded for the Phase 12 rewind fixed-MAX decision. The
            // assert is a sanity ceiling only (RAM images ≈ 420KB).
            eprintln!("payload {name}: {} bytes", state.len());
            assert!(
                state.len() < 2 * 1024 * 1024,
                "payload {name} too large: {}",
                state.len()
            );
        }
    }

    #[test]
    fn continued_emulation_matches_uninterrupted_run() {
        // End-to-end fidelity: load-then-run must equal never-having-saved.
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(
            &rom(),
            &CoreConfig {
                region: None,
                bios_paths: HashMap::new(),
                controllers: HashMap::new(),
                core_options: None,
            },
        )
        .unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
            240,
            160,
            nerust_render_traits::PixelFormat::Rgba,
        );
        core.render_frame(&mut frame).unwrap();
        let saved = core.save_state().unwrap();
        core.render_frame(&mut frame).unwrap();
        core.render_frame(&mut frame).unwrap();
        let reference = core.save_state().unwrap();
        core.load_state(&saved).unwrap();
        core.render_frame(&mut frame).unwrap();
        core.render_frame(&mut frame).unwrap();
        assert_eq!(core.save_state().unwrap(), reference);
    }

    #[test]
    fn load_state_rejects_options_mismatch() {
        use crate::core_options::GbaCoreOptions;

        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
        };
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(&rom(), &config).unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
            240,
            160,
            nerust_render_traits::PixelFormat::Rgba,
        );
        core.render_frame(&mut frame).unwrap();
        let saved = core.save_state().unwrap();
        // Reload the same ROM under a different solar level: the payload
        // pins the old options, so import must refuse atomically.
        let options = GbaCoreOptions {
            solar_light_level: 0x10,
        };
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: Some(Box::new(options)),
        };
        core.load(&rom(), &config).unwrap();
        assert!(core.load_state(&saved).is_err());
        // The refused import changed nothing: a fresh export validates
        // against the current (new-options) system.
        let fresh = core.save_state().unwrap();
        core.load_state(&fresh).unwrap();
    }

    #[test]
    fn machine_state_round_trips_battery_backed_cartridge() {
        fn sram_rom() -> Vec<u8> {
            let mut rom = rom();
            let tag = b"SRAM_V00";
            rom[0x1000..0x1000 + tag.len()].copy_from_slice(tag);
            rom
        }
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: None,
        };
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(&sram_rom(), &config).unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
            240,
            160,
            nerust_render_traits::PixelFormat::Rgba,
        );
        core.render_frame(&mut frame).unwrap();
        // Battery RAM plus an attached GPIO device travel in the envelope.
        core.loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .write8(0x0E000123, 0x5A);
        core.loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .cartridge_mut()
            .unwrap()
            .gpio
            .write(0x080000C8, 2, 1);
        let saved = core.save_state().unwrap();
        core.loaded
            .as_mut()
            .unwrap()
            .system
            .bus
            .write8(0x0E000123, 0x00);
        core.render_frame(&mut frame).unwrap();
        core.load_state(&saved).unwrap();
        // NOTE: no bus reads before the re-export below. `bus.read8` is
        // not side-effect-free: it accumulates wait cycles and advances
        // the N/S trackers, so any read would legitimately perturb the
        // exported bytes.
        assert_eq!(core.save_state().unwrap(), saved);
        let loaded = core.loaded.as_mut().unwrap();
        assert_eq!(loaded.system.bus.read8(0x0E000123), 0x5A);
        assert!(
            loaded
                .system
                .bus
                .cartridge_mut()
                .unwrap()
                .gpio
                .is_attached()
        );
    }

    #[test]
    fn rejects_machine_state_from_another_rom() {
        let mut a = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        a.load(
            &rom(),
            &CoreConfig {
                region: None,
                bios_paths: HashMap::new(),
                controllers: HashMap::new(),
                core_options: None,
            },
        )
        .unwrap();
        let state = a.save_state().unwrap();
        let mut b = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        let mut rom2 = rom();
        rom2[0x100] ^= 1;
        crate::cartridge::header::finalize_test_gba_rom(&mut rom2);
        b.load(
            &rom2,
            &CoreConfig {
                region: None,
                bios_paths: HashMap::new(),
                controllers: HashMap::new(),
                core_options: None,
            },
        )
        .unwrap();
        assert!(b.load_state(&state).is_err());
    }

    #[test]
    fn restart_audio_starts_backend() {
        use std::sync::atomic::Ordering::SeqCst;

        use nerust_core_traits::audio::{AudioBackend, StereoSample};

        struct StartProbe {
            started: Arc<AtomicBool>,
        }
        impl AudioBackend for StartProbe {
            fn start(&mut self) {
                self.started.store(true, SeqCst);
            }
            fn pause(&mut self) {}
            fn push(&mut self, _sample: StereoSample) {}
        }
        let started = Arc::new(AtomicBool::new(false));
        let mut core = GbaConsoleCore::new(
            Box::new(StartProbe {
                started: started.clone(),
            }),
            test_emu_input(),
        );
        core.restart_audio();
        assert!(started.load(SeqCst));
    }
}
