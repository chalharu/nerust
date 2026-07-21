pub mod audio;
pub mod debugger;
pub mod factory;
pub mod identity;
pub mod memory_space;
pub mod save_state;
pub mod touch;

use std::{collections::HashMap, fmt::Debug, io, path::PathBuf, sync::mpsc::Sender};

use downcast_rs::Downcast;
use dyn_clone::DynClone;
use dyn_eq::DynEq;
use nerust_render_traits::{FrameBuffer, PixelFormat};
use serde::{Deserialize, Serialize};

use crate::audio::AudioBackend;
use crate::debugger::Debugger;
use nerust_input_traits::ControllerHub;

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
    #[error("invalid core options")]
    InvalidCoreOptions,
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
    pub core_options: Option<Box<dyn DynCoreOptions>>,
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

    // -- debug I/O (default: not supported) --
    /// Runs one frame with externally-provided controller and audio backend.
    ///
    /// Unlike `render_frame()` which manages controller and audio internally,
    /// this method lets the caller inject custom I/O — used by `rom_test`
    /// and future TAS / netplay tools.
    ///
    /// Returns the total CPU cycles elapsed during the frame.
    fn render_frame_with_io(
        &mut self,
        _frame_slot: &mut FrameBuffer,
        _controller: &mut dyn ControllerHub,
        _audio: &mut dyn AudioBackend,
    ) -> Result<u64, CoreError> {
        Err(CoreError::Core(Box::new(io::Error::other(
            "render_frame_with_io not supported",
        ))))
    }

    /// Creates a debugger object for inspecting internal state.
    ///
    /// Returns `None` if debugging is not supported by this core.
    /// The returned debugger shares internal state with the console core
    /// and must not outlive it.
    fn create_debugger(&mut self) -> Option<Box<dyn Debugger>> {
        None
    }
}

pub trait CoreOptions:
    Serialize + for<'de> Deserialize<'de> + Debug + Clone + Eq + Send + Sync + 'static
{
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOptionsWrapper<T: CoreOptions>(T);

pub trait DynCoreOptions: DynClone + Debug + DynEq + Downcast + Send + Sync {}

impl<T: CoreOptions> DynCoreOptions for CoreOptionsWrapper<T> {}

downcast_rs::impl_downcast!(DynCoreOptions);
dyn_clone::clone_trait_object!(DynCoreOptions);
dyn_eq::eq_trait_object!(DynCoreOptions);

impl<T: CoreOptions> From<T> for Box<dyn DynCoreOptions> {
    fn from(value: T) -> Self {
        Box::new(CoreOptionsWrapper(value))
    }
}

pub trait DynCoreOptionsExt: Sized {
    fn into_inner<T: CoreOptions>(self) -> Result<T, Self>;
}

impl DynCoreOptionsExt for Box<dyn DynCoreOptions> {
    fn into_inner<T: CoreOptions>(self) -> Result<T, Self> {
        self.downcast::<CoreOptionsWrapper<T>>()
            .map(|wrapper| wrapper.0)
            .map_err(|boxed| boxed as Box<dyn DynCoreOptions>)
    }
}
