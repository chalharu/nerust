use nerust_core_traits::{ConsoleCore, CoreCapabilities, CoreConfig, CoreError, VideoSignalKind};
use nerust_core_traits::identity::SystemIdentity;
use nerust_render_traits::{FrameBuffer, PixelFormat};
use crate::rom_identity::{GbaRomIdentity};
use crate::rom_identity::GbaSystemId;

pub struct GbaConsoleCore {
    loaded: bool,
    paused: bool,
    rom_identity: Option<GbaRomIdentity>,
}

impl GbaConsoleCore {
    pub fn new() -> Self {
        Self {
            loaded: false,
            paused: false,
            rom_identity: None,
        }
    }
}

impl Default for GbaConsoleCore {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleCore for GbaConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![PixelFormat::Rgba],
            video_signal: VideoSignalKind::Lcd,
        }
    }

    fn render_frame(&mut self, _frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        if !self.loaded {
            return Err(CoreError::NoRomLoaded);
        }
        // Phase 10 で実装
        Ok(())
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        self.rom_identity = GbaRomIdentity::from_rom(rom);
        self.loaded = true;
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = false;
        self.rom_identity = None;
    }

    fn reset(&mut self) {
        // Phase 10 で実装
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        // Phase 10 で実装
        Ok(Vec::new())
    }

    fn load_state(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        // Phase 10 で実装
        Ok(())
    }

    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        Ok(SystemIdentity {
            system_id: Box::new(GbaSystemId),
            identity_bytes: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_correct() {
        let core = GbaConsoleCore::new();
        let caps = core.capabilities();
        assert_eq!(caps.output_formats.len(), 1);
        assert!(matches!(caps.video_signal, VideoSignalKind::Lcd));
    }

    #[test]
    fn default_state() {
        let core = GbaConsoleCore::new();
        assert!(!core.paused());
    }
}
