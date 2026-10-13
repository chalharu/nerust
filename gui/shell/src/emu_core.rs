use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use nerust_core_traits::{
    CoreConfig, CoreOptions, EmuCommand, LoadCommand, StateDataCommand,
    debugger::{
        DebugImage, DebuggerError, InspectError, InspectRequest, InspectResult, MemoryWrite,
        SpaceInfo, StepUnit,
    },
    factory::{CoreParts, load::MediaObject},
    identity::SystemIdentity,
};
use nerust_emu_thread::{ConsoleMetrics, EmuThread, OperationError};
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput};
use nerust_render_traits::{FrameBuffer, PixelFormat, VideoFrameFormat, VideoRenderProfile};

use crate::session::commands::SlotOpFailure;

/// Errors from core operations invoked by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum CorePersistenceError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Core(String),
}

impl CorePersistenceError {
    /// Best-effort UX classification of a state import/export failure.
    /// Only drives the user-facing message; the full detail stays in
    /// the error itself (and the logs).
    pub fn slot_failure(&self) -> SlotOpFailure {
        match self {
            Self::WorkerUnavailable | Self::NoReply => SlotOpFailure::Unavailable,
            Self::Core(message) => {
                let message = message.to_lowercase();
                if message.contains("mismatch")
                    || message.contains("unsupported")
                    || message.contains("version")
                {
                    SlotOpFailure::Incompatible
                } else {
                    SlotOpFailure::Corrupt
                }
            }
        }
    }
}

/// Bound for debugger query round-trips (inspect, disassembly,
/// images, steps, writes). These run on the UI thread: an unbounded
/// `recv` would freeze every window's input forever if the emu thread
/// ever stalls (wedged audio, a panicked core that already dropped
/// its reply). A timeout degrades to the existing loud `NoReply`
/// failure instead, which every drain already reports in the status
/// line. Five seconds is generous: the dearest path (a running GBA
/// image re-decode) answers in milliseconds.
const DEBUG_REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn recv_debug_reply<T>(reply_rx: std::sync::mpsc::Receiver<T>) -> Result<T, OperationError> {
    reply_rx.recv_timeout(DEBUG_REPLY_TIMEOUT).map_err(|error| {
        match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => {
                log::warn!("debugger round-trip timed out; emu thread may be stalled");
            }
            std::sync::mpsc::RecvTimeoutError::Disconnected => {}
        }
        OperationError::NoReply
    })
}
/// The persistence-relevant subset of EmuCore's interface.
pub trait CorePersistence {
    fn save_state_raw(&self) -> Result<Vec<u8>, CorePersistenceError>;
    fn load_state_raw(&self, data: Vec<u8>) -> Result<(), CorePersistenceError>;
    fn generate_preview(&self) -> Option<crate::state::PreviewFrame>;
    fn canonical_media_identity(&self) -> Option<SystemIdentity>;
    fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, CorePersistenceError>;
    fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), CorePersistenceError>;
}

/// Thread-transport port behind [`EmuCore`]. The application layer
/// (session, drains) programs against this seam, not the concrete
/// [`EmuThread`]: call sites never assume thread internals, and tests
/// plug scripted transports without spawning threads. The production
/// reality is one implementation; the trait exists so the direction
/// of the dependency (application → abstraction) stays honest.
pub trait EmuThreadPort: Send + std::fmt::Debug {
    fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::TrySendError<EmuCommand>>;
    fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>>;
    fn frame_count(&self) -> u64;
    fn fps(&self) -> f32;
}

impl EmuThreadPort for EmuThread {
    fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::TrySendError<EmuCommand>> {
        EmuThread::send(self, cmd)
    }

    fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
        EmuThread::shared_frame_buffer(self)
    }

    fn frame_count(&self) -> u64 {
        EmuThread::frame_count(self)
    }

    fn fps(&self) -> f32 {
        EmuThread::fps(self)
    }
}

pub struct EmuCore {
    emu: Box<dyn EmuThreadPort>,
    render_profile: VideoRenderProfile,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    disp_fb: FrameBuffer,
    frame_ready: Arc<AtomicBool>,
    metrics: Arc<Mutex<ConsoleMetrics>>,
    image_cache: Mutex<CachedImages>,
}

/// Cached system images (pattern tables) for the debugger PPU viewer.
///
/// Decode is the drain's dearest pass (GBA re-reads 131k bus bytes
/// per fetch) while paused memory never changes, so repeats serve
/// from here. Anything that advances or replaces memory — step,
/// write, load/unload/reset/state import, or a running core —
/// marks the cache dirty instead. `generation` bumps on every dirty mark:
/// PPU viewers compare generations instead of hashing megabytes.
#[derive(Debug, Default)]
struct CachedImages {
    dirty: bool,
    generation: u64,
    images: Vec<DebugImage>,
}

impl EmuCore {
    #[doc(hidden)]
    pub fn new(
        emu: Box<dyn EmuThreadPort>,
        render_profile: VideoRenderProfile,
        shared_fb: Arc<Mutex<FrameBuffer>>,
        disp_fb: FrameBuffer,
        frame_ready: Arc<AtomicBool>,
    ) -> Self {
        let metrics = Arc::new(Mutex::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        }));
        Self {
            emu,
            render_profile,
            shared_fb,
            disp_fb,
            frame_ready,
            metrics,
            image_cache: Mutex::new(CachedImages {
                dirty: true,
                generation: 0,
                images: Vec::new(),
            }),
        }
    }

    /// Wrap `CoreParts` (from a factory) into an `EmuCore`.
    /// Returns (EmuCore, GuiInput, field_map).
    ///
    /// Buffer geometry comes from the factory-declared render profile
    /// (size + frame format), never per-system constants: indexed
    /// systems carry their palette, direct-color systems (Rgba) carry
    /// none. The thread slot is built identically so all three buffers
    /// agree before the first swap.
    pub fn from_parts(
        parts: CoreParts,
    ) -> (
        Self,
        GuiInput,
        std::collections::HashMap<(AttachmentId, DigitalControlId), usize>,
        nerust_core_traits::peripheral::HostPeripheralHandles,
    ) {
        let field_map = parts.field_map;
        use std::sync::Mutex;
        let src_w = parts.render_profile.source_logical_size.width;
        let src_h = parts.render_profile.source_logical_size.height;
        let pixel_format = match parts.render_profile.frame_format {
            VideoFrameFormat::Palette => PixelFormat::PaletteIndex {
                palette: parts.palette.clone(),
            },
            VideoFrameFormat::Rgba => PixelFormat::Rgba,
        };

        let new_buffer = || {
            let mut fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format.clone());
            fb.resize(src_w, src_h);
            fb
        };
        let shared_fb = Arc::new(Mutex::new(new_buffer()));
        let disp_fb = new_buffer();
        let frame_slot = new_buffer();

        let frame_ready = Arc::new(AtomicBool::new(false));
        let emu = EmuThread::spawn(
            parts.core,
            Arc::clone(&shared_fb),
            Arc::clone(&frame_ready),
            frame_slot,
            parts.audio,
        );
        (
            Self {
                emu: Box::new(emu),
                render_profile: parts.render_profile,
                shared_fb,
                disp_fb,
                frame_ready,
                metrics: Arc::new(Mutex::new(ConsoleMetrics {
                    paused: true,
                    ..ConsoleMetrics::default()
                })),
                image_cache: Mutex::new(CachedImages {
                    dirty: true,
                    generation: 0,
                    images: Vec::new(),
                }),
            },
            parts.gui_input,
            field_map,
            parts.host_peripherals,
        )
    }

    pub fn render_profile(&self) -> &VideoRenderProfile {
        &self.render_profile
    }

    pub fn swap_frame_buffer(&mut self) {
        if self.frame_ready.load(Ordering::Relaxed)
            && let Ok(mut guard) = self.shared_fb.lock()
            && self.frame_ready.swap(false, Ordering::AcqRel)
        {
            std::mem::swap(&mut *guard, &mut self.disp_fb);
        }
        if let Ok(mut guard) = self.metrics.lock() {
            guard.frame_counter = self.emu.frame_count();
        } else {
            log::warn!("metrics lock poisoned in swap_frame_buffer");
        }
    }

    pub fn frame_buffer(&self) -> &FrameBuffer {
        &self.disp_fb
    }

    pub fn clear_display(&mut self) {
        self.disp_fb.clear();
        if let Ok(mut guard) = self.shared_fb.lock() {
            guard.clear();
        }
        self.frame_ready.store(false, Ordering::Release);
    }

    // metrics() のみ into_inner() で回復する理由: レンダリングパスで毎フレーム呼ばれ、
    // 常に最新の metrics を返す必要がある。他のメソッド(swap, pause, load 等)の metrics
    // 更新は副次的な副作用であり、poison 時に諦めても動作に影響しない。
    pub fn metrics(&self) -> ConsoleMetrics {
        let mut guard = self.metrics.lock().unwrap_or_else(|e| {
            log::warn!("metrics mutex poisoned, recovering");
            e.into_inner()
        });
        guard.frame_counter = self.emu.frame_count();
        guard.emulation_fps = self.emu.fps();
        *guard
    }

    pub fn set_volume(&self, volume: f32) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::SetVolume(volume))
            .map_err(|_| OperationError::WorkerUnavailable)
    }

    /// Re-acquire the audio backend stream on the emu thread (idempotent;
    /// the thread also restarts the rate-control filter from nominal).
    pub fn restart_audio(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::RestartAudio)
            .map_err(|_| OperationError::WorkerUnavailable)
    }

    pub fn resume(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Resume)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = false,
            Err(e) => log::warn!("metrics lock poisoned in resume: {e}"),
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Pause)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = true,
            Err(e) => log::warn!("metrics lock poisoned in pause: {e}"),
        }
        Ok(())
    }

    pub fn load(
        &self,
        media: &MediaObject,
        core_options: Option<Box<dyn CoreOptions>>,
    ) -> Result<(), OperationError> {
        self.load_inner(media, core_options, false)
    }

    /// Load and start paused: no free-run frame executes before the
    /// first command. Headless drivers use this for deterministic
    /// frame zero; interactive loads keep free-running.
    pub fn load_paused(
        &self,
        media: &MediaObject,
        core_options: Option<Box<dyn CoreOptions>>,
    ) -> Result<(), OperationError> {
        self.load_inner(media, core_options, true)
    }

    fn load_inner(
        &self,
        media: &MediaObject,
        core_options: Option<Box<dyn CoreOptions>>,
        start_paused: bool,
    ) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Load(Box::new(LoadCommand {
                rom: media.bytes.as_ref().to_vec(),
                config: CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                    core_options,
                    // Stamped authoritatively by the emu thread from the
                    // owned backend; None here just means "not yet known".
                    audio_sample_rate: None,
                },
                start_paused,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()));
        if result.is_ok() {
            match self.metrics.lock() {
                Ok(mut guard) => {
                    guard.loaded = true;
                    guard.paused = true;
                }
                Err(e) => log::warn!("metrics lock poisoned in load: {e}"),
            }
            self.mark_images_dirty();
        }
        result
    }

    /// Mark cached system images stale. Called by every path that
    /// advances or replaces memory; the next `debug_images` refetches.
    /// The generation bumps so viewers can compare generations.
    fn mark_images_dirty(&self) {
        if let Ok(mut cache) = self.image_cache.lock() {
            cache.dirty = true;
            cache.generation = cache.generation.wrapping_add(1);
        }
    }

    pub fn unload(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Unload)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.loaded = false,
            Err(e) => log::warn!("metrics lock poisoned in unload: {e}"),
        }
        self.mark_images_dirty();
        Ok(())
    }

    pub fn reset(&self) -> Result<(), OperationError> {
        let result = self
            .emu
            .send(EmuCommand::Reset)
            .map_err(|_| OperationError::WorkerUnavailable);
        if result.is_ok() {
            self.mark_images_dirty();
        }
        result
    }

    pub fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::MapperSave { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()))
    }

    pub fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::ImportMapperSave(Box::new(StateDataCommand {
                data: bytes,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()));
        if result.is_ok() {
            self.mark_images_dirty();
        }
        result
    }

    /// Raw state save: Send SaveState, return core bytes (no header, no preview).
    pub fn save_state_raw(&self) -> Result<Vec<u8>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SaveState { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()))
    }

    /// Generate a preview frame from the EmuThread's shared frame buffer.
    pub fn generate_preview(&self) -> Option<crate::state::PreviewFrame> {
        generate_preview_frame(&*self.emu)
    }

    /// Advance execution by one unit. The reply is the barrier: it arrives
    /// after exactly one step. Returns thread errors outer, domain step
    /// errors inner.
    pub fn step(&self, unit: StepUnit) -> Result<Result<u64, DebuggerError>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Step {
                unit,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = recv_debug_reply(reply_rx);
        if matches!(result, Ok(Ok(_))) {
            self.mark_images_dirty();
        }
        result
    }

    /// On-demand inspect through the emu thread. Returns thread errors
    /// outer, inspect errors (including `NotPaused`) inner.
    pub fn inspect(
        &self,
        req: InspectRequest,
    ) -> Result<Result<InspectResult, InspectError>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerInspect {
                req,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
    }

    /// On-demand disassembly through the emu thread. Cold path:
    /// callers hold pause for frame-consistent rows.
    pub fn disassemble(
        &self,
        addr: u32,
        count: u16,
    ) -> Result<Result<Vec<nerust_core_traits::debugger::DisasmLine>, DebuggerError>, OperationError>
    {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerDisassemble {
                addr,
                count,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
    }

    /// On-demand system images through the emu thread. Same cold-path
    /// contract as [`EmuCore::disassemble`], plus a pause-frozen
    /// cache: repeats while paused serve the stored decode, so page
    /// and refresh drains skip the tile re-read. A running core
    /// advances memory at any time and always refetches. Returns the
    /// source generation with the pixels: viewers skip re-deriving
    /// when the generation they already show comes back.
    pub fn debug_images(
        &self,
    ) -> Result<
        Result<(u64, Vec<nerust_core_traits::debugger::DebugImage>), DebuggerError>,
        OperationError,
    > {
        if !self.metrics().paused {
            self.mark_images_dirty();
        }
        let cached = self
            .image_cache
            .lock()
            .ok()
            .filter(|cache| !cache.dirty && !cache.images.is_empty())
            .map(|cache| (cache.generation, cache.images.clone()));
        if let Some((generation, images)) = cached {
            return Ok(Ok((generation, images)));
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerImages { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = recv_debug_reply(reply_rx);
        let images = match result {
            Ok(Ok(images)) => images,
            Ok(Err(error)) => return Ok(Err(error)),
            Err(_) => return Err(OperationError::NoReply),
        };
        // Poison recovery mirrors `metrics()`: a poisoned cache still
        // serves this decode instead of failing the viewer.
        let mut cache = self.image_cache.lock().unwrap_or_else(|e| {
            log::warn!("image cache mutex poisoned, recovering");
            e.into_inner()
        });
        cache.images.clone_from(&images);
        cache.dirty = false;
        Ok(Ok((cache.generation, images)))
    }

    /// On-demand program counter through the emu thread. Ungated (a
    /// single register read); `Ok(None)` means no anchor.
    pub fn program_counter(&self) -> Result<Result<Option<u32>, DebuggerError>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerPc { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
    }

    /// Memory edit through the emu thread. Pause-ungated by design;
    /// deterministic callers pause first.
    pub fn write_memory(
        &self,
        req: MemoryWrite,
    ) -> Result<Result<(), DebuggerError>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::WriteMemory {
                req,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = recv_debug_reply(reply_rx);
        if matches!(result, Ok(Ok(()))) {
            self.mark_images_dirty();
        }
        result
    }

    /// Register edit through the emu thread. Same pause-ungated
    /// contract as [`EmuCore::write_memory`].
    pub fn write_register(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<Result<(), DebuggerError>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::WriteRegister {
                name,
                value,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = recv_debug_reply(reply_rx);
        if matches!(result, Ok(Ok(()))) {
            self.mark_images_dirty();
        }
        result
    }

    /// Install a nominal-audio tap: every rendered frame clones its
    /// caller-buffer samples into `tap`. Headless capture only.
    pub fn tap_nominal_audio(
        &self,
        tap: Arc<Mutex<Vec<nerust_core_traits::audio::StereoSample>>>,
    ) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::TapNominalAudio {
                tap,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx.recv().map_err(|_| OperationError::NoReply)
    }

    /// Install a guest-output tap for one channel: every rendered
    /// frame moves the core's freshly produced bytes for that channel
    /// into `tap`. Headless capture only.
    pub fn tap_serial_output(
        &self,
        channel: &str,
        tap: Arc<Mutex<Vec<u8>>>,
    ) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::TapSerialOutput {
                channel: channel.to_string(),
                tap,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx.recv().map_err(|_| OperationError::NoReply)
    }

    /// Memory-space table snapshot through the emu thread. Empty when
    /// idle or when the core exposes no debugger. Static metadata:
    /// no pause gating required.
    pub fn memory_spaces(&self) -> Result<Vec<SpaceInfo>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerSpaces { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
    }

    /// Guest-output channel names through the emu thread. Empty when
    /// idle. Static metadata: answered without pause gating.
    pub fn output_channels(&self) -> Result<Vec<String>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::OutputChannels { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
    }

    pub fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Identity { reply: reply_tx })
            .map_err(|_| {
                log::warn!("canonical_media_identity: emu thread unavailable");
            })
            .ok()?;
        match reply_rx.recv() {
            Ok(Ok(identity)) => Some(identity),
            Ok(Err(e)) => {
                log::warn!("canonical_media_identity: core error: {e}");
                None
            }
            Err(_) => {
                log::warn!("canonical_media_identity: reply channel closed");
                None
            }
        }
    }

    /// Raw state load: Send LoadState with raw core bytes. No format fallback.
    pub fn load_state_raw(&self, data: Vec<u8>) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::LoadState(Box::new(StateDataCommand {
                data,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()));
        if result.is_ok() {
            self.mark_images_dirty();
        }
        result
    }
}

impl CorePersistence for EmuCore {
    fn save_state_raw(&self) -> Result<Vec<u8>, CorePersistenceError> {
        self.save_state_raw().map_err(op_to_persistence)
    }

    fn load_state_raw(&self, data: Vec<u8>) -> Result<(), CorePersistenceError> {
        self.load_state_raw(data).map_err(op_to_persistence)
    }

    fn generate_preview(&self) -> Option<crate::state::PreviewFrame> {
        self.generate_preview()
    }

    fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        self.canonical_media_identity()
    }

    fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, CorePersistenceError> {
        self.save_mapper_raw().map_err(op_to_persistence)
    }

    fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), CorePersistenceError> {
        self.load_mapper_raw(bytes).map_err(op_to_persistence)
    }
}

fn op_to_persistence(e: OperationError) -> CorePersistenceError {
    match e {
        OperationError::WorkerUnavailable => CorePersistenceError::WorkerUnavailable,
        OperationError::NoReply => CorePersistenceError::NoReply,
        OperationError::Reply(s) => CorePersistenceError::Core(s),
    }
}

/// Generate a preview frame from the port's shared frame buffer.
/// Caller should hold no lock on the shared frame buffer.
///
/// Lives here (not in `state`) so the module graph stays acyclic:
/// `state` owns the [`crate::state::PreviewFrame`] DTO, `emu_core`
/// owns the port it reads from — neither imports the other for
/// behavior.
fn generate_preview_frame(emu: &dyn EmuThreadPort) -> Option<crate::state::PreviewFrame> {
    let Ok(guard) = emu.shared_frame_buffer().lock() else {
        log::warn!("generate_preview: shared frame buffer lock failed");
        return None;
    };
    let w = guard.width();
    let h = guard.height();
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = if let Some(palette) = guard.palette() {
        let indices = guard.as_ref();
        let mut rgba = Vec::with_capacity(w * h * 4);
        for &idx in indices.iter().take(w * h) {
            let color = palette[idx as usize];
            rgba.push((color >> 24) as u8);
            rgba.push((color >> 16) as u8);
            rgba.push((color >> 8) as u8);
            rgba.push(color as u8);
        }
        rgba
    } else {
        compact_rgba(&guard)?
    };
    drop(guard);
    Some(crate::state::PreviewFrame {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

fn compact_rgba(frame: &FrameBuffer) -> Option<Vec<u8>> {
    let row_bytes = frame.width().checked_mul(4)?;
    let source_len = frame.stride().checked_mul(frame.height())?;
    let source = frame.as_ref().get(..source_len)?;
    if frame.stride() < row_bytes {
        return None;
    }

    let mut rgba = Vec::with_capacity(row_bytes.checked_mul(frame.height())?);
    for row in source.chunks_exact(frame.stride()) {
        rgba.extend_from_slice(&row[..row_bytes]);
    }
    Some(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_core_parts;
    use nerust_core_traits::debugger::SpaceId;

    #[test]
    fn compact_rgba_removes_aligned_row_padding() {
        let mut frame = FrameBuffer::with_capacity(3, 2, PixelFormat::Rgba);
        frame.resize(3, 2);
        assert!(frame.stride() > 3 * 4);
        frame.as_mut().fill(0xEE);
        frame.as_mut()[..12].copy_from_slice(&[1; 12]);
        let second_row = frame.stride();
        frame.as_mut()[second_row..second_row + 12].copy_from_slice(&[2; 12]);

        assert_eq!(compact_rgba(&frame), Some([[1; 12], [2; 12]].concat()));
    }

    fn loaded_core() -> EmuCore {
        let (core, _, _, _) = EmuCore::from_parts(build_test_core_parts());
        let media = MediaObject::new(None, vec![0u8; 16]);
        core.load(&media, None).expect("mock ROM loads");
        assert!(core.metrics().paused);
        core
    }

    /// Scripted transport: answers one query, refuses the rest. Proves
    /// `EmuCore` programs against the port, never thread internals —
    /// no thread spawns here.
    #[derive(Debug)]
    struct FakePort {
        spaces: Vec<SpaceInfo>,
    }

    impl EmuThreadPort for FakePort {
        fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::TrySendError<EmuCommand>> {
            match cmd {
                EmuCommand::DebuggerSpaces { reply } => {
                    let _ = reply.send(self.spaces.clone());
                    Ok(())
                }
                other => Err(mpsc::TrySendError::Disconnected(other)),
            }
        }

        fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
            panic!("no buffers behind the scripted port")
        }

        fn frame_count(&self) -> u64 {
            0
        }

        fn fps(&self) -> f32 {
            0.0
        }
    }

    #[test]
    fn port_seam_serves_queries_without_a_thread() {
        use nerust_core_traits::debugger::{SpaceAccess, SpaceInfo};
        let parts = build_test_core_parts();
        let port = FakePort {
            spaces: vec![SpaceInfo {
                id: SpaceId(0),
                key: "wram",
                name: "WRAM",
                address_bits: 8,
                range: 0x0000..=0x00FF,
                access: SpaceAccess::ReadWrite,
            }],
        };
        let picture = || {
            let mut fb = FrameBuffer::with_capacity(2, 1, PixelFormat::Rgba);
            fb.resize(2, 1);
            fb
        };
        let core = EmuCore::new(
            Box::new(port),
            parts.render_profile,
            Arc::new(Mutex::new(picture())),
            picture(),
            Arc::new(AtomicBool::new(false)),
        );
        let spaces = core.memory_spaces().expect("port answers");
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].name, "WRAM");
    }

    #[test]
    fn debug_images_cache_serves_paused_repeats() {
        let core = loaded_core();
        let (gen0, images0) = core.debug_images().expect("thread").expect("images");
        assert_eq!(images0.len(), 1);
        let (gen1, images1) = core.debug_images().expect("thread").expect("images");
        assert_eq!((gen0, images1.len()), (gen1, 1));
        core.write_memory(MemoryWrite {
            space: SpaceId(0),
            addr: 0x10,
            width: 1,
            value: 0xFF,
        })
        .expect("thread")
        .expect("writable");
        let (gen2, _) = core.debug_images().expect("thread").expect("images");
        assert!(gen2 > gen1, "writes dirty the cache generation");
        // A running core always refetches: the metrics mirror says
        // running, so the generation bumps on every call.
        core.resume().expect("thread");
        let (gen3, _) = core.debug_images().expect("thread").expect("images");
        assert!(gen3 > gen2, "running refetches advance the generation");
        core.pause().expect("thread");
    }

    #[test]
    fn debug_registers_roundtrip_and_refuse() {
        let core = loaded_core();
        core.write_register("a", 5)
            .expect("thread")
            .expect("a writable");
        let inspected = core
            .inspect(InspectRequest {
                space: Some(SpaceId(0)),
                addr: Some(0),
                rows: 1,
            })
            .expect("thread")
            .expect("inspect");
        assert_eq!(inspected.registers[0], ("a", 5));
        assert!(matches!(
            core.write_register("bogus", 0),
            Ok(Err(DebuggerError::Unsupported))
        ));
    }

    #[test]
    fn debug_observation_paths_answer() {
        let core = loaded_core();
        assert_eq!(core.memory_spaces().expect("thread").len(), 1);
        assert_eq!(
            core.program_counter().expect("thread").expect("pc"),
            Some(0x10)
        );
        assert_eq!(
            core.disassemble(0x10, 2)
                .expect("thread")
                .expect("rows")
                .len(),
            2
        );
        assert!(
            core.step(StepUnit::Frame).expect("thread").expect("step") >= 1,
            "frame steps report the barrier frame counter"
        );
    }
}
