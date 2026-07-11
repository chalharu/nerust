pub mod audio;
pub mod factory;
pub mod identity;
pub mod save_state;
pub mod touch;

use std::{collections::HashMap, path::PathBuf, sync::mpsc::Sender};

use nerust_render_base::{FrameBuffer, PixelFormat};

// ---------------------------------------------------------------------------
// CoreError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    RomParse(Box<dyn std::error::Error + Send + Sync>),
    #[error("{0}")]
    Core(Box<dyn std::error::Error + Send + Sync>),
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
    /// System-specific options (e.g. serialized `CoreOptions` for NES).
    /// Interpreted by the `ConsoleCore` implementation.
    pub core_options: Vec<u8>,
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
    fn render_frame(&mut self, frame_slot: &mut FrameBuffer) -> Result<(), CoreError>;

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
