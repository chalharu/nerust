pub mod audio;
pub mod debugger;
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
    RomParse(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("{0}")]
    Core(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("no ROM loaded")]
    NoRomLoaded,
    #[error("invalid core options")]
    InvalidCoreOptions,
    /// Structured persistence refusal. Concrete cores attach the original error as source.
    #[error("{source}")]
    Persistence {
        kind: CoreFault,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Application-relevant fault kind, independent of diagnostic wording or a concrete core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreFault {
    Unavailable,
    Storage,
    Missing,
    Corrupt,
    Incompatible,
}

impl CoreError {
    pub fn fault(&self) -> CoreFault {
        match self {
            Self::Persistence { kind, .. } => *kind,
            Self::NoRomLoaded => CoreFault::Unavailable,
            Self::InvalidCoreOptions => CoreFault::Incompatible,
            Self::RomParse(_) => CoreFault::Corrupt,
            Self::Core(error) => match error.downcast_ref::<std::io::Error>() {
                Some(error) if error.kind() == std::io::ErrorKind::NotFound => CoreFault::Missing,
                Some(_) => CoreFault::Storage,
                None => CoreFault::Corrupt,
            },
        }
    }
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
    /// Live audio-backend swap between frames. No pause, no reload;
    /// on GBA no rate-boundary state crossing.
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
    /// Advance execution by one unit. The reply is the frame counter
    /// after the stepped frame (barrier position). Callers pause first;
    /// stepping while running races free-run.
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
    /// Re-basis nominal audio production onto a new device rate, once
    /// per swap between frames. Only the rate basis changes; voice and
    /// machine state are untouched.
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
    /// Static channel topology: names are core-defined, never invented
    /// by generic drivers. PAIR RULE: override with
    /// `take_channel_bytes` together, or reads fail loudly.
    fn output_channels(&self) -> Vec<String> {
        Vec::new()
    }
    /// Drain bytes produced on `channel` since the last call (once per
    /// frame per tapped channel). Empty by default and for unknown
    /// channels.
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

#[cfg(test)]
mod tests {
    use super::{
        ConsoleCore, CoreCapabilities, CoreConfig, CoreError, SwapAudioCommand, VideoSignalKind,
        audio,
    };
    use nerust_render_traits::FrameBuffer;

    #[derive(Debug, Default)]
    struct StubCore {
        paused: bool,
    }

    impl ConsoleCore for StubCore {
        fn capabilities(&self) -> CoreCapabilities {
            CoreCapabilities {
                output_formats: Vec::new(),
                video_signal: VideoSignalKind::Other,
            }
        }

        fn render_frame(
            &mut self,
            _frame_slot: &mut FrameBuffer,
            _audio_out: &mut Vec<audio::StereoSample>,
        ) -> Result<(), CoreError> {
            Err(CoreError::NoRomLoaded)
        }

        fn load(&mut self, _rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
            Err(CoreError::NoRomLoaded)
        }

        fn unload(&mut self) {}

        fn reset(&mut self) {
            self.paused = false;
        }

        fn paused(&self) -> bool {
            self.paused
        }

        fn set_paused(&mut self, paused: bool) {
            self.paused = paused;
        }

        fn set_audio_sample_rate(&mut self, _sample_rate: u32) {}

        fn save_state(&self) -> Result<Vec<u8>, CoreError> {
            Err(CoreError::NoRomLoaded)
        }

        fn load_state(&mut self, _data: &[u8]) -> Result<(), CoreError> {
            Err(CoreError::NoRomLoaded)
        }
    }

    #[test]
    fn default_channels_are_empty_and_identity_fails_without_rom() {
        let mut core = StubCore::default();
        assert_eq!(core.mapper_save().unwrap(), None);
        assert!(core.import_mapper_save(&[]).is_ok());
        assert!(core.debugger().is_none());
        assert!(core.debug_control().is_none());
        assert!(core.output_channels().is_empty());
        assert!(core.take_channel_bytes("main").is_empty());
        assert!(matches!(core.identity(), Err(CoreError::NoRomLoaded)));
        assert_eq!(core.rewind_state_size(), None);
        core.set_paused(true);
        assert!(core.paused());
        // Required lifecycle methods stay callable on the stub.
        let config = CoreConfig {
            region: None,
            bios_paths: std::collections::HashMap::new(),
            controllers: std::collections::HashMap::new(),
            core_options: None,
            audio_sample_rate: None,
        };
        assert!(matches!(
            core.load(&[], &config),
            Err(CoreError::NoRomLoaded)
        ));
        let mut frame =
            FrameBuffer::with_capacity(256, 240, nerust_render_traits::PixelFormat::Rgba);
        let mut audio_out = Vec::new();
        assert!(matches!(
            core.render_frame(&mut frame, &mut audio_out),
            Err(CoreError::NoRomLoaded)
        ));
        assert!(matches!(core.save_state(), Err(CoreError::NoRomLoaded)));
        assert!(matches!(core.load_state(&[]), Err(CoreError::NoRomLoaded)));
        core.unload();
        core.reset();
        assert!(!core.paused());
    }

    #[test]
    #[should_panic(expected = "rewind not supported")]
    fn rewind_save_panics_without_support() {
        StubCore::default().rewind_save(&mut [0; 8]);
    }

    #[test]
    #[should_panic(expected = "rewind not supported")]
    fn rewind_restore_panics_without_support() {
        StubCore::default().rewind_restore(&[0; 8]);
    }

    #[test]
    fn core_errors_render_stable_messages() {
        assert_eq!(format!("{}", CoreError::NoRomLoaded), "no ROM loaded");
        assert_eq!(
            format!("{}", CoreError::InvalidCoreOptions),
            "invalid core options"
        );
    }

    #[test]
    fn core_fault_classification_uses_types_and_preserves_sources() {
        use super::CoreFault;
        use std::error::Error as _;

        let error = CoreError::Persistence {
            kind: CoreFault::Incompatible,
            source: "truncated data without compatibility keywords".into(),
        };
        assert_eq!(error.fault(), CoreFault::Incompatible);
        assert!(error.source().is_some());
        assert_eq!(CoreError::NoRomLoaded.fault(), CoreFault::Unavailable);
        assert_eq!(
            CoreError::InvalidCoreOptions.fault(),
            CoreFault::Incompatible
        );
        let error = CoreError::Core(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "unsupported version mismatch",
        )));
        assert_eq!(error.fault(), CoreFault::Missing);
        assert_eq!(
            CoreError::Core("io version mismatch".into()).fault(),
            CoreFault::Corrupt
        );
    }

    #[test]
    fn swap_audio_command_debug_hides_backend_but_shows_rate() {
        use std::sync::mpsc::channel;

        #[derive(Debug)]
        struct StubBackend;
        impl audio::AudioBackend for StubBackend {
            fn start(&mut self) {}
            fn pause(&mut self) {}
            fn push(&mut self, _sample: audio::StereoSample) {}
        }

        let (reply, _rx) = channel();
        let command = SwapAudioCommand {
            backend: Box::new(StubBackend),
            sample_rate: 48_000,
            reply,
        };
        let rendered = format!("{command:?}");
        assert!(rendered.contains("48000"), "{rendered}");
    }

    #[test]
    fn stereo_sample_math_matches_scalar_expectations() {
        let stereo = audio::StereoSample::new(0.5, -0.25);
        assert_eq!(stereo.downmix(), 0.125);
        assert_eq!(stereo.scale(2.0).downmix(), 0.25);
        assert_eq!(audio::StereoSample::mono(0.5).downmix(), 0.5);
    }
}
