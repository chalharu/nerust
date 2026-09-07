use std::sync::Arc;

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind, audio::AudioBackend,
    identity::SystemIdentity,
};
use nerust_input_traits::EmuInput;
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{
    input_types::GbaInputBuffer, rom_identity::GbaRomIdentity, rom_identity::GbaSystemId,
    system::GbaSystem,
};

#[derive(Debug, thiserror::Error)]
enum GbaCoreError {
    #[error("invalid or unsupported GBA ROM")]
    InvalidRom,
    #[error("GBA input buffer has the wrong concrete type")]
    InvalidInputBuffer,
    #[error("save state ROM mismatch")]
    RomMismatch,
}

struct LoadedGba {
    system: GbaSystem,
    rom: Arc<[u8]>,
    identity: GbaRomIdentity,
}

pub struct GbaConsoleCore {
    loaded: Option<LoadedGba>,
    audio: Box<dyn AudioBackend>,
    emu_input: EmuInput,
    paused: bool,
}

impl GbaConsoleCore {
    pub fn new(audio: Box<dyn AudioBackend>, emu_input: EmuInput) -> Self {
        Self {
            loaded: None,
            audio,
            emu_input,
            paused: false,
        }
    }

    pub fn new_empty(audio: Box<dyn AudioBackend>, emu_input: EmuInput) -> Self {
        Self::new(audio, emu_input)
    }

    fn create_loaded(rom: &[u8]) -> Result<LoadedGba, CoreError> {
        let identity = GbaRomIdentity::from_rom(rom)
            .ok_or_else(|| CoreError::RomParse(Box::new(GbaCoreError::InvalidRom)))?;
        let system = GbaSystem::from_test_rom(rom.to_vec())
            .or_else(|| GbaSystem::from_rom(rom.to_vec()))
            .ok_or_else(|| CoreError::RomParse(Box::new(GbaCoreError::InvalidRom)))?;
        Ok(LoadedGba {
            system,
            rom: Arc::from(rom),
            identity,
        })
    }

    fn loaded_ref(&self) -> Result<&LoadedGba, CoreError> {
        self.loaded.as_ref().ok_or(CoreError::NoRomLoaded)
    }

    fn loaded_mut(&mut self) -> Result<&mut LoadedGba, CoreError> {
        self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)
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
            .ok_or_else(|| CoreError::Core(Box::new(GbaCoreError::InvalidInputBuffer)))?
            .0;
        let loaded = self.loaded.as_mut().ok_or(CoreError::NoRomLoaded)?;
        loaded.system.bus.set_keyinput(input);
        // Run one LCD frame (228 lines * 1232 cycles)
        for _ in 0..280_896 {
            if loaded.system.step_tcycle() {
                break;
            }
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
            let src_bytes = unsafe {
                std::slice::from_raw_parts(src_row.as_ptr() as *const u8, 240 * 4)
            };
            let dst_offset = y * stride;
            dst[dst_offset..dst_offset + 240 * 4].copy_from_slice(src_bytes);
        }
        Ok(())
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        let loaded = Self::create_loaded(rom)?;
        self.loaded = Some(loaded);
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
        if let Ok(reset) = Self::create_loaded(&current.rom) {
            self.loaded = Some(reset);
        }
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let loaded = self.loaded_ref()?;
        // Minimal state: ROM bytes + identity hash. Full bus serialization is Phase 10.
        // Include rom len and first 16 bytes as identity check.
        let mut out = Vec::with_capacity(loaded.rom.len() + 32);
        out.extend_from_slice(&(loaded.rom.len() as u32).to_le_bytes());
        out.extend_from_slice(&loaded.rom);
        Ok(out)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let loaded = self.loaded_ref()?;
        if data.len() < 4 {
            return Err(CoreError::Core(Box::new(GbaCoreError::RomMismatch)));
        }
        let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + len || len != loaded.rom.len() {
            return Err(CoreError::Core(Box::new(GbaCoreError::RomMismatch)));
        }
        if &data[4..4 + len] != &*loaded.rom {
            return Err(CoreError::Core(Box::new(GbaCoreError::RomMismatch)));
        }
        // For minimal implementation, reload from ROM (full bus state not yet serialized)
        let rom = loaded.rom.clone();
        let new_loaded = Self::create_loaded(&rom).map_err(|e| CoreError::Core(Box::new(e)))?;
        self.loaded = Some(new_loaded);
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) {
        self.audio.set_volume(volume);
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let _loaded = self.loaded_ref()?;
        Ok(None)
    }

    fn import_mapper_save(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
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
    fn load_render_and_state_round_trip() {
        let mut core = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        core.load(&rom(), &CoreConfig { region: None, bios_paths: HashMap::new(), controllers: HashMap::new(), core_options: None }).unwrap();
        let mut frame = nerust_render_traits::FrameBuffer::with_capacity(240, 160, nerust_render_traits::PixelFormat::Rgba);
        core.render_frame(&mut frame).unwrap();
        assert_eq!((frame.width(), frame.height()), (240, 160));
        let state = core.save_state().unwrap();
        core.load_state(&state).unwrap();
    }

    #[test]
    fn rejects_machine_state_from_another_rom() {
        let mut a = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        a.load(&rom(), &CoreConfig { region: None, bios_paths: HashMap::new(), controllers: HashMap::new(), core_options: None }).unwrap();
        let state = a.save_state().unwrap();
        let mut b = GbaConsoleCore::new(Box::new(NullAudio), test_emu_input());
        let mut rom2 = rom();
        rom2[0x100] ^= 1;
        crate::cartridge::header::finalize_test_gba_rom(&mut rom2);
        b.load(&rom2, &CoreConfig { region: None, bios_paths: HashMap::new(), controllers: HashMap::new(), core_options: None }).unwrap();
        assert!(b.load_state(&state).is_err());
    }
}
