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

/// Boxed payload for `EmuCommand::SwapAudio`. Boxed like the other
/// large payloads so the enum stays small. `backend` carries no
/// `Debug` impl (audio backends never needed one), so this struct
/// formats as the rate only — enough to identify a swap in logs.
pub struct SwapAudioCommand {
    /// Replacement backend, built session-side from the new settings
    /// (volume/gain already applied). The thread `start()`s it on swap
    /// and `pause()`s the old one: backend lifecycle stays thread-owned.
    pub backend: Box<dyn audio::AudioBackend + Send>,
    /// Device rate of `backend` (`backend.sample_rate()` at build).
    /// Stamped into the core alongside the swap so nominal production
    /// and the device agree again (ratio back near 1.0).
    pub sample_rate: u32,
    pub reply: Sender<()>,
}

impl std::fmt::Debug for SwapAudioCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwapAudioCommand")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

/// Boxed payload for `EmuCommand::SwapFrameSlot`. The thread owns its
/// render target exclusively, so even a same-size palette refresh must
/// cross as a command: the reply is the barrier proving the thread
/// parked on the new slot before the session swaps its own buffers.
#[derive(Debug)]
pub struct SwapFrameSlotCommand {
    /// Fresh render target (geometry + pixel format + palette), built
    /// session-side from the replacement video assets.
    pub slot: FrameBuffer,
    pub reply: Sender<()>,
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
    /// Live audio-backend swap for sample-rate/latency changes. Runs
    /// synchronously between frames: pause the old backend, start the
    /// new one, stamp `sample_rate` into the core, reset the
    /// rate-control filter. No pause needed, no ROM reload, no state
    /// round-trip — and on GBA no `AudioRateMismatch`, since no state
    /// crosses a rate boundary at all.
    SwapAudio(Box<SwapAudioCommand>),
    /// Live render-target swap for presentation-only changes (video
    /// filter). Runs synchronously between frames; callers pause first
    /// so no frame straddles the session-side buffer swap that follows
    /// the reply.
    SwapFrameSlot(Box<SwapFrameSlotCommand>),
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
    /// On-demand disassembly. Cold path like [`EmuCommand::DebuggerInspect`]:
    /// callers hold pause for frame-consistent rows; the thread does not
    /// gate, since decoding is side-effect free by kernel contract.
    DebuggerDisassemble {
        addr: u32,
        count: u16,
        reply: Sender<Result<Vec<debugger::DisasmLine>, debugger::DebuggerError>>,
    },
    /// On-demand system images (pattern tables). Same cold-path contract
    /// as [`EmuCommand::DebuggerDisassemble`].
    DebuggerImages {
        reply: Sender<Result<Vec<debugger::DebugImage>, debugger::DebuggerError>>,
    },
    /// On-demand program counter for disassembly anchoring. Answers
    /// without pause gating (a single register read); `Ok(None)` means
    /// the system names no anchor.
    DebuggerPc {
        reply: Sender<Result<Option<u32>, debugger::DebuggerError>>,
    },
    /// Batched debugger snapshot (inspect + anchor + disassembly) in
    /// one crossing. Same per-part contracts as the individual
    /// commands; prefer over three sequential round trips wherever
    /// the caller needs the whole set (debugger drains).
    DebuggerSnapshot {
        req: debugger::SnapshotRequest,
        reply: Sender<Result<debugger::SnapshotResult, debugger::DebuggerError>>,
    },
    /// Memory edit through the control path. Deliberately pause-ungated:
    /// `DebuggerError` has no not-paused variant by design (§5.2), so
    /// deterministic callers pause first and racing writes interleave
    /// with free-run by contract.
    WriteMemory {
        req: debugger::MemoryWrite,
        reply: Sender<Result<(), debugger::DebuggerError>>,
    },
    /// Register edit through the control path. `name` is one of the
    /// backend's register names (`&'static str` travels the channel
    /// without allocation). Same pause-ungated contract as
    /// [`EmuCommand::WriteMemory`].
    WriteRegister {
        name: &'static str,
        value: u64,
        reply: Sender<Result<(), debugger::DebuggerError>>,
    },
    /// Install a nominal-audio tap: every rendered frame clones its
    /// caller-buffer samples into `tap`. Headless capture only; the
    /// session audio transport is unaffected.
    TapNominalAudio {
        tap: Arc<Mutex<Vec<audio::StereoSample>>>,
        reply: Sender<()>,
    },
    /// Install a guest-output tap for one named channel (see
    /// `ConsoleCore::output_channels`): every rendered frame moves the
    /// core's freshly produced bytes for that channel into `tap`
    /// (overwriting the previous frame's delta; accumulation is the
    /// reader's job). Headless capture only.
    TapSerialOutput {
        channel: String,
        tap: Arc<Mutex<Vec<u8>>>,
        reply: Sender<()>,
    },
    /// Memory-space table snapshot. Static metadata: answered without
    /// pause gating; empty when idle or when the core exposes no
    /// debugger. Lets generic drivers resolve stable space keys
    /// without naming system tables.
    DebuggerSpaces {
        reply: Sender<Vec<debugger::SpaceInfo>>,
    },
    /// Guest-output channel names. Static metadata: answered without
    /// pause gating; empty when idle. Lets generic drivers install one
    /// tap per channel without naming system channels.
    OutputChannels {
        reply: Sender<Vec<String>>,
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

    // -- audio device rate --
    /// Re-basis nominal audio production onto a new device rate without
    /// reconstructing the core. The session calls this exactly once per
    /// `EmuCommand::SwapAudio`, between frames, after swapping the
    /// backend: re-derive anything rate-baked (resamplers, filter
    /// coefficients, cached DSP states) here. Voice/machine state is
    /// untouched — only the rate basis changes.
    fn set_audio_sample_rate(&mut self, sample_rate: u32);

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

    // -- guest output channels (default: none) --
    /// Names of the guest-output byte channels this core produces
    /// (e.g. a link-cable serial stream, a guest debug-log sink).
    /// Static topology: names are core-defined, data (manifests) may
    /// reference them, generic drivers never invent them.
    ///
    /// Override PAIR checklist: `output_channels` and
    /// `take_channel_bytes` must be overridden together. Listing a
    /// channel without draining it (or vice versa) fails loudly at
    /// read time, never at compile time, so keep the pair in sync.
    fn output_channels(&self) -> Vec<String> {
        Vec::new()
    }
    /// Drain bytes the core produced on `channel` since the last call.
    /// The session layer calls this once per rendered frame per tapped
    /// channel; accumulation is the reader's job. Line-oriented sinks
    /// flatten to newline-delimited bytes here (levels/prefixes are
    /// the assert side's concern, if ever needed). Empty by default and
    /// for unknown channels: cores stay silent unless they produce.
    fn take_channel_bytes(&mut self, _channel: &str) -> Vec<u8> {
        Vec::new()
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
