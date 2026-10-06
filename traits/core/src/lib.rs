pub mod audio;
pub mod debugger;
pub mod factory;
pub mod identity;
pub mod peripheral;
pub mod save_state;
pub mod touch;

use std::{
    collections::HashMap,
    fmt::Debug,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::Sender},
};

use downcast_rs::Downcast;
use dyn_clone::DynClone;
use nerust_render_traits::{FrameBuffer, PixelFormat};

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
    pub core_options: Option<Box<dyn CoreOptions>>,
    /// Device sample rate for the core's internal resamplers, stamped by
    /// the emu thread (which owns the audio backend) on `Load`. Cores
    /// never touch the backend; they only need the rate as data.
    /// `None` (unit tests) falls back to 48kHz.
    pub audio_sample_rate: Option<u32>,
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
    /// Start paused: no free-run frame executes between load and the
    /// first command. Headless drivers need this for deterministic
    /// frame zero; interactive sessions leave it false.
    pub start_paused: bool,
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
    /// Re-acquire the audio backend stream (idempotent). Mobile OS
    /// lifecycle transitions can kill audio streams; frontends send
    /// this on foreground resume.
    RestartAudio,
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
    /// Advance execution by one unit. Processed synchronously when the
    /// command is drained, so receipt is the frame barrier: each reply
    /// corresponds to exactly one executed step. Callers pause first
    /// for determinism; stepping while running races free-run.
    ///
    /// The success value is the frame counter after the stepped frame
    /// (barrier position), not a cycle count: the thread renders the
    /// frame itself so pixels, tap, and swap stay on the shared path.
    /// Cycle counts remain available through `DebugControl::step`
    /// for direct-console drivers.
    Step {
        unit: debugger::StepUnit,
        reply: Sender<Result<u64, debugger::DebuggerError>>,
    },
    /// On-demand memory/panel inspection. Requires pause (§5.1): reading
    /// while running cannot guarantee the values match the displayed frame.
    DebuggerInspect {
        req: debugger::InspectRequest,
        reply: Sender<Result<debugger::InspectResult, debugger::InspectError>>,
    },
    /// Memory edit through the control path. Deliberately pause-ungated:
    /// `DebuggerError` has no not-paused variant by design (§5.2), so
    /// deterministic callers pause first and racing writes interleave
    /// with free-run by contract.
    WriteMemory {
        req: debugger::MemoryWrite,
        reply: Sender<Result<(), debugger::DebuggerError>>,
    },
    /// Install a nominal-audio tap: every rendered frame clones its
    /// caller-buffer samples into `tap`. Headless capture only; the
    /// session audio transport is unaffected.
    TapNominalAudio {
        tap: Arc<Mutex<Vec<audio::StereoSample>>>,
        reply: Sender<()>,
    },
    /// Memory-space table snapshot. Static metadata: answered without
    /// pause gating; empty when idle or when the core exposes no
    /// debugger. Lets generic drivers resolve stable space keys
    /// without naming system tables.
    DebuggerSpaces {
        reply: Sender<Vec<debugger::SpaceInfo>>,
    },
}

// ---------------------------------------------------------------------------
// ConsoleCore trait
// ---------------------------------------------------------------------------

pub trait ConsoleCore: Send + Downcast {
    // -- video + audio production --
    fn capabilities(&self) -> CoreCapabilities;
    /// Run one frame: video into `frame_slot`, nominal-rate audio into
    /// `audio_out` (implementations clear it first, then append the
    /// frame's samples). Transport (rate control, backend push) is the
    /// session layer's job: cores never see the backend.
    fn render_frame(
        &mut self,
        frame_slot: &mut FrameBuffer,
        audio_out: &mut Vec<audio::StereoSample>,
    ) -> Result<(), CoreError>;

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

    // -- mapper save (system-specific, default: not supported) --
    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(None)
    }
    fn import_mapper_save(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }

    // -- debugger (default: not supported) --
    /// Read-only observer. `&self` guarantees non-invasive observation.
    /// Returns `None` when no ROM is loaded or the core has no debugger.
    fn debugger(&self) -> Option<Box<dyn debugger::Debugger + '_>> {
        None
    }
    /// Execution control and memory editing. `None` when unsupported.
    /// Observation via `debugger()` stays available independently.
    fn debug_control(&mut self) -> Option<Box<dyn debugger::DebugControl + '_>> {
        None
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

pub trait CoreOptions: Debug + DynClone + Downcast + Send {}

downcast_rs::impl_downcast!(CoreOptions);
downcast_rs::impl_downcast!(ConsoleCore);
dyn_clone::clone_trait_object!(CoreOptions);

impl<T: CoreOptions> From<T> for Box<dyn CoreOptions> {
    fn from(value: T) -> Self {
        Box::new(value)
    }
}
