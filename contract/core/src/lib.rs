pub mod audio;
pub mod channel;
pub mod device;
pub mod identity;
pub mod input;
pub mod persistence;
pub mod save_state;

pub use save_state::{SaveStateHeader, load_state_from_header, save_state_with_header};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

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

#[derive(Clone, Debug)]
pub enum GpuCommand {
    Blit { slot: u32 },
    PaletteDecode { slot: u32 },
}

#[derive(Clone, Debug)]
pub struct GpuCommandList {
    pub commands: Vec<GpuCommand>,
}

// ---------------------------------------------------------------------------
// PixelFormat (re-export from screen/video for convenience)
// ---------------------------------------------------------------------------

pub use nerust_screen_video::{FrameBuffer, PixelFormat};

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

/// Boxed payload for `EmuCommand::Load`. Keeps the enum small (~16 bytes).
#[derive(Debug)]
pub struct LoadCommand {
    pub rom: Vec<u8>,
    pub config: CoreConfig,
    pub reply: Sender<Result<(), CoreError>>,
}

/// Boxed payload for `EmuCommand::LoadState` / `EmuCommand::ImportMapperSave`.
#[derive(Debug)]
pub struct StateDataCommand {
    pub data: Vec<u8>,
    pub reply: Sender<Result<(), CoreError>>,
}

#[derive(Debug)]
pub enum EmuCommand {
    Pause,
    Resume,
    Reset,
    Quit,
    Load(Box<LoadCommand>),
    Unload,
    SetVolume(f32),
    SaveState {
        reply: Sender<Result<Vec<u8>, CoreError>>,
    },
    LoadState(Box<StateDataCommand>),
    MapperSave {
        reply: Sender<Result<Option<Vec<u8>>, CoreError>>,
    },
    ImportMapperSave(Box<StateDataCommand>),
    Identity {
        reply: Sender<Result<identity::SystemIdentity, CoreError>>,
    },
}

// ---------------------------------------------------------------------------
// ConsoleCore trait
// ---------------------------------------------------------------------------

pub trait ConsoleCore: Send {
    // -- video --
    fn capabilities(&self) -> CoreCapabilities;
    fn render_frame(&mut self, frame_slot: &mut FrameBuffer) -> Result<GpuCommandList, CoreError>;

    // -- peripherals --
    fn attach_device(&mut self, port: usize, device: Box<dyn device::Device>);
    fn detach_device(&mut self, port: usize);

    // -- lifecycle --
    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError>;
    fn unload(&mut self);
    fn reset(&mut self);

    // -- audio --
    fn set_volume(&mut self, _volume: f32) {}

    // -- pause --
    fn paused(&self) -> bool;
    fn set_paused(&mut self, paused: bool);

    // -- save states --
    fn save_state(&self) -> Result<Vec<u8>, CoreError>;
    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError>;

    // -- mapper save (system-specific, default: not supported) --
    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(None)
    }
    fn import_mapper_save(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }

    // -- identity --
    fn identity(&self) -> Result<identity::SystemIdentity, CoreError> {
        Err(CoreError::NoRomLoaded)
    }

    // -- rewind (default: not supported) --
    /// Returns `None` if rewind is not supported.
    fn rewind_state_size(&self) -> Option<usize> {
        None
    }
    /// Saves the current state into `buf` for rewind.
    ///
    /// # Panics
    /// Panics if the core does not support rewind.
    /// Check `rewind_state_size()` returns `Some` before calling.
    fn rewind_save(&self, _buf: &mut [u8]) {
        panic!("rewind not supported")
    }
    /// Restores a previously saved rewind state.
    ///
    /// # Panics
    /// Panics if the core does not support rewind.
    /// Check `rewind_state_size()` returns `Some` before calling.
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
            ],
        };
        assert_eq!(list.commands.len(), 2);
    }
}
