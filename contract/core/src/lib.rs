pub mod audio;
pub mod channel;
pub mod device;
pub mod input;
pub mod mirror;
pub mod options;
pub mod persistence;
pub mod rom;

use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// CoreError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("ROM parse failed: {0}")]
    RomParse(String),
    #[error("core error: {0}")]
    Core(String),
    #[error("no ROM loaded")]
    NoRomLoaded,
}

// ---------------------------------------------------------------------------
// VideoSignalKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSignalKind {
    Ntsc,
    Rgb,
    Lcd,
    Other,
}

// ---------------------------------------------------------------------------
// CoreCapabilities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CoreCapabilities {
    pub output_formats: Vec<PixelFormat>,
    pub video_signal: VideoSignalKind,
}

// ---------------------------------------------------------------------------
// GpuCommand
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum GpuCommand {
    Blit { slot: u32 },
    PaletteDecode { slot: u32 },
    UploadPalette { slot: u32 },
}

#[derive(Clone)]
pub struct GpuCommandList {
    pub commands: Vec<GpuCommand>,
}

// ---------------------------------------------------------------------------
// PixelFormat (re-export from screen/video for convenience)
// ---------------------------------------------------------------------------

pub use nerust_screen_video::PixelFormat;

// ---------------------------------------------------------------------------
// Region
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Ntsc,
    Pal,
}

// ---------------------------------------------------------------------------
// ControllerKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerKind {
    None,
    Standard,
    Zapper,
}

// ---------------------------------------------------------------------------
// CoreConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub region: Option<Region>,
    pub bios_paths: HashMap<String, PathBuf>,
    pub controllers: HashMap<usize, ControllerKind>,
}

// ---------------------------------------------------------------------------
// EmuCommand
// ---------------------------------------------------------------------------

pub enum EmuCommand {
    RenderFrame,
    Pause,
    Resume,
    Reset,
    Quit,
}

// ---------------------------------------------------------------------------
// ConsoleCore trait
// ---------------------------------------------------------------------------

pub trait ConsoleCore: Send {
    // -- video --
    fn capabilities(&self) -> CoreCapabilities;
    fn render_frame(&mut self, frame_slot: &mut [u8]) -> Result<GpuCommandList, CoreError>;
    fn frame_slot_size(&self) -> usize;

    // -- audio --
    fn audio_samples(&self, out: &mut dyn audio::AudioBackend);

    // -- peripherals --
    fn attach_device(&mut self, port: usize, device: Box<dyn device::Device>);
    fn detach_device(&mut self, port: usize);

    // -- lifecycle --
    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError>;
    fn unload(&mut self);
    fn reset(&mut self);

    // -- pause --
    fn paused(&self) -> bool;
    fn set_paused(&mut self, paused: bool);

    // -- save states --
    fn save_state(&self) -> Result<Vec<u8>, CoreError>;
    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError>;

    // -- rewind (default: not supported) --
    fn rewind_state_size(&self) -> Option<usize> {
        None
    }
    fn rewind_save(&self, _buf: &mut [u8]) {
        panic!("rewind not supported")
    }
    fn rewind_restore(&mut self, _buf: &[u8]) {
        panic!("rewind not supported")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_command_round_trips_slot_number() {
        let cmd = GpuCommand::Blit { slot: 42 };
        match cmd {
            GpuCommand::Blit { slot } => assert_eq!(slot, 42),
            _ => panic!("expected Blit"),
        }
    }

    #[test]
    fn gpu_command_list_holds_commands() {
        let list = GpuCommandList {
            commands: vec![
                GpuCommand::Blit { slot: 0 },
                GpuCommand::PaletteDecode { slot: 1 },
                GpuCommand::UploadPalette { slot: 2 },
            ],
        };
        assert_eq!(list.commands.len(), 3);
    }
}
