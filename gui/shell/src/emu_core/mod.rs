use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use nerust_core_traits::{
    CoreConfig, CoreOptions, EmuCommand, LoadCommand, StateDataCommand, SwapAudioCommand,
    SwapFrameSlotCommand,
    audio::AudioBackend,
    debugger::{
        DebuggerError, InspectError, InspectRequest, InspectResult, MemoryWrite, SpaceInfo,
        StepUnit,
    },
    factory::{CoreParts, load::MediaObject},
    identity::SystemIdentity,
};
use nerust_emu_thread::{ConsoleMetrics, EmuThread, OperationError};
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput};
use nerust_render_traits::{FrameBuffer, PixelFormat, VideoFrameFormat, VideoRenderProfile};

use crate::session::commands::SlotOpFailure;

pub(crate) mod images;

use self::images::CachedImages;

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

/// Bound for debugger query round-trips. Interactive drains never wait
/// on it (async flights degrade per-await instead); it only bounds the
/// blocking wrappers for tests, headless drivers, and non-drain callers.
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

/// Thread-transport port behind [`EmuCore`]: application code
/// programs against this seam, tests plug scripted transports.
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

/// Clone shares the transport, metrics, frame slot, and image cache
/// (`Arc`s); the display framebuffer is reset to empty instead of
/// copied. Rationale: clones exist for debugger drain flights, which
/// never touch `disp_fb` (no swap, no blit) — copying 61-164KB per
/// drain begin would be pure waste.
impl Clone for EmuCore {
    fn clone(&self) -> Self {
        Self {
            emu: Arc::clone(&self.emu),
            render_profile: self.render_profile.clone(),
            shared_fb: Arc::clone(&self.shared_fb),
            disp_fb: FrameBuffer::with_capacity(0, 0, PixelFormat::Rgba),
            frame_ready: Arc::clone(&self.frame_ready),
            metrics: Arc::clone(&self.metrics),
            image_cache: Arc::clone(&self.image_cache),
        }
    }
}

pub struct EmuCore {
    emu: Arc<dyn EmuThreadPort>,
    render_profile: VideoRenderProfile,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    disp_fb: FrameBuffer,
    frame_ready: Arc<AtomicBool>,
    metrics: Arc<Mutex<ConsoleMetrics>>,
    image_cache: Arc<Mutex<CachedImages>>,
}

impl EmuCore {
    #[doc(hidden)]
    pub fn new(
        emu: Arc<dyn EmuThreadPort>,
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
            image_cache: Arc::new(Mutex::new(CachedImages {
                dirty: true,
                generation: 0,
                images: Vec::new(),
                served_hash: None,
                served_generation: 0,
            })),
        }
    }

    /// Wrap `CoreParts` into an `EmuCore`. Buffer geometry comes from
    /// the factory render profile so all buffers agree pre-swap.
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
        let new_buffer = || fresh_frame_buffer(&parts.render_profile, &parts.palette);
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
                emu: Arc::new(emu),
                render_profile: parts.render_profile,
                shared_fb,
                disp_fb,
                frame_ready,
                metrics: Arc::new(Mutex::new(ConsoleMetrics {
                    paused: true,
                    ..ConsoleMetrics::default()
                })),
                image_cache: Arc::new(Mutex::new(CachedImages {
                    dirty: true,
                    generation: 0,
                    images: Vec::new(),
                    served_hash: None,
                    served_generation: 0,
                })),
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

    /// Shareable clone for the async debug drain: owns a clone and
    /// polls across UI ticks. Shared `Arc`s see the same pause truth
    /// and image generation; `disp_fb` stays empty (unused).
    pub(crate) fn drain_core(&self) -> EmuCore {
        self.clone()
    }

    /// Live audio-backend swap. Blocks for the reply barrier; no pause,
    /// no reload. Volume-only deltas ride with the new backend.
    pub fn swap_audio_backend(
        &self,
        backend: Box<dyn AudioBackend>,
        sample_rate: u32,
    ) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SwapAudio(Box::new(SwapAudioCommand {
                backend,
                sample_rate,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
    }

    /// Live presentation swap for filter-only changes: pause if running,
    /// swap behind the reply barrier, resume if paused. No ROM reload,
    /// no state round-trip; dirties the debugger image cache.
    pub fn apply_presentation(
        &mut self,
        profile: VideoRenderProfile,
        palette: Box<[u32]>,
    ) -> Result<(), OperationError> {
        let was_running = !self.metrics().paused;
        if was_running {
            self.pause()?;
        }
        let slot_result = self.swap_frame_slot(fresh_frame_buffer(&profile, palette.as_ref()));
        if slot_result.is_err() {
            if was_running {
                let _ = self.resume();
            }
            return slot_result;
        }
        match self.shared_fb.lock() {
            Ok(mut guard) => {
                *guard = fresh_frame_buffer(&profile, palette.as_ref());
            }
            Err(error) => {
                log::warn!("presentation swap: shared frame buffer lock failed: {error}");
                if was_running {
                    let _ = self.resume();
                }
                return Err(OperationError::Reply(
                    "shared frame buffer lock poisoned during presentation swap".to_string(),
                ));
            }
        }
        self.disp_fb = fresh_frame_buffer(&profile, palette.as_ref());
        self.render_profile = profile;
        self.frame_ready.store(false, Ordering::Release);
        self.mark_images_dirty();
        if was_running {
            self.resume()?;
        }
        Ok(())
    }

    /// Exchange the thread-owned render target behind a reply barrier.
    /// The caller swaps the session-side buffers only after this
    /// returns (and only while paused), so no frame straddles the two
    /// swaps. Kept private: session code goes through
    /// [`EmuCore::apply_presentation`], which owns the choreography.
    fn swap_frame_slot(&self, slot: FrameBuffer) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SwapFrameSlot(Box::new(SwapFrameSlotCommand {
                slot,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        recv_debug_reply(reply_rx)
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
        let reply_rx = self.step_send(unit)?;
        let result = recv_debug_reply(reply_rx);
        if matches!(result, Ok(Ok(_))) {
            self.mark_images_dirty();
        }
        result
    }

    /// Send half of [`EmuCore::step`]: the command queues without
    /// blocking, and the caller polls the receiver on its own tick.
    /// The async debug drain pairs this with `mark_images_dirty` on
    /// success, mirroring the blocking body above.
    pub(crate) fn step_send(
        &self,
        unit: StepUnit,
    ) -> Result<mpsc::Receiver<Result<u64, DebuggerError>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Step {
                unit,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
    }

    /// On-demand inspect through the emu thread. Returns thread errors
    /// outer, inspect errors (including `NotPaused`) inner.
    pub fn inspect(
        &self,
        req: InspectRequest,
    ) -> Result<Result<InspectResult, InspectError>, OperationError> {
        recv_debug_reply(self.inspect_send(req)?)
    }

    /// Send half of [`EmuCore::inspect`]: queues without blocking so
    /// the async debug drain can poll the receiver on its own tick.
    pub(crate) fn inspect_send(
        &self,
        req: InspectRequest,
    ) -> Result<mpsc::Receiver<Result<InspectResult, InspectError>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerInspect {
                req,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
    }

    /// Batched debugger snapshot (inspect + anchor + disassembly)
    /// in one thread crossing. Same per-part contracts as the
    /// individual calls; prefer wherever the caller needs the whole
    /// set (debugger drains) over three sequential timeouts.
    pub fn debug_snapshot_batch(
        &self,
        req: nerust_core_traits::debugger::SnapshotRequest,
    ) -> Result<Result<nerust_core_traits::debugger::SnapshotResult, DebuggerError>, OperationError>
    {
        recv_debug_reply(self.snapshot_batch_send(req)?)
    }

    /// Send half of [`EmuCore::debug_snapshot_batch`]: queues without
    /// blocking so the async debug drain can poll on its own tick.
    pub(crate) fn snapshot_batch_send(
        &self,
        req: nerust_core_traits::debugger::SnapshotRequest,
    ) -> Result<
        mpsc::Receiver<Result<nerust_core_traits::debugger::SnapshotResult, DebuggerError>>,
        OperationError,
    > {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerSnapshot {
                req,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
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
        let result = recv_debug_reply(self.write_memory_send(req)?);
        if matches!(result, Ok(Ok(()))) {
            self.mark_images_dirty();
        }
        result
    }

    /// Send half of [`EmuCore::write_memory`]: queues without blocking.
    /// The async debug drain pairs this with `mark_images_dirty` on
    /// success, mirroring the blocking body above.
    pub(crate) fn write_memory_send(
        &self,
        req: MemoryWrite,
    ) -> Result<mpsc::Receiver<Result<(), DebuggerError>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::WriteMemory {
                req,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
    }

    /// Register edit through the emu thread. Same pause-ungated
    /// contract as [`EmuCore::write_memory`].
    pub fn write_register(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<Result<(), DebuggerError>, OperationError> {
        let result = recv_debug_reply(self.write_register_send(name, value)?);
        if matches!(result, Ok(Ok(()))) {
            self.mark_images_dirty();
        }
        result
    }

    /// Send half of [`EmuCore::write_register`]: queues without
    /// blocking (dirty-marking stays with the poller, as above).
    pub(crate) fn write_register_send(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<mpsc::Receiver<Result<(), DebuggerError>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::WriteRegister {
                name,
                value,
                reply: reply_tx,
            })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
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

/// Fresh render target for one profile + palette pair. Single
/// construction path for core spawn and live presentation swaps, so a
/// swapped filter can never disagree with a rebuilt one on geometry.
fn fresh_frame_buffer(profile: &VideoRenderProfile, palette: &[u32]) -> FrameBuffer {
    let pixel_format = match profile.frame_format {
        VideoFrameFormat::Palette => PixelFormat::PaletteIndex {
            palette: palette.to_vec().into_boxed_slice(),
        },
        VideoFrameFormat::Rgba => PixelFormat::Rgba,
    };
    let src_w = profile.source_logical_size.width;
    let src_h = profile.source_logical_size.height;
    let mut fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format);
    fb.resize(src_w, src_h);
    fb
}

/// Preview frame from the port's shared frame buffer. Caller holds
/// no lock on it. Lives here to keep the module graph acyclic.
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
    use nerust_core_traits::debugger::{DebugImage, SpaceId};

    #[serial_test::serial]
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

    #[serial_test::serial]
    #[test]
    fn apply_presentation_swaps_buffers_and_profile_on_running_core() {
        let (mut core, _, _, _) = EmuCore::from_parts(build_test_core_parts());
        core.resume().expect("resume for pause choreography");
        assert!(!core.metrics().paused);

        // NTSC-like assets: same source geometry, wider logical size,
        // distinct palette.
        let mut profile = core.render_profile().clone();
        profile.logical_size.width = 602;
        let palette: Box<[u32]> = (0..256).map(|i| 0xFF00_0000 | i).collect();
        core.apply_presentation(profile, palette).expect("swap ok");

        // Choreography restored the running state behind the barrier.
        assert!(!core.metrics().paused, "running core resumes after swap");
        assert_eq!(core.render_profile().logical_size.width, 602);
        assert_eq!(
            core.frame_buffer()
                .palette()
                .expect("palette rides the buffer")[0xAB],
            0xFF00_00AB,
            "display buffer carries the new palette"
        );
        // Shared slot agrees (next swap publishes new-format pixels).
        let guard = core.emu.shared_frame_buffer().lock().unwrap();
        assert_eq!(guard.palette().expect("shared palette")[0xAB], 0xFF00_00AB);
    }

    #[serial_test::serial]
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
            Arc::new(port),
            parts.render_profile,
            Arc::new(Mutex::new(picture())),
            picture(),
            Arc::new(AtomicBool::new(false)),
        );
        let spaces = core.memory_spaces().expect("port answers");
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].name, "WRAM");
    }

    #[serial_test::serial]
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
        // The mock decode never changes, so the refetch serves the
        // same pixels under the served generation: viewers skip
        // re-deriving instead of churning on a bump.
        let (gen2, images2) = core.debug_images().expect("thread").expect("images");
        assert_eq!(images2.len(), 1);
        assert_eq!(gen2, gen1, "identical content keeps the served generation");
        // A running core always refetches; static content still keeps
        // the served generation.
        core.resume().expect("thread");
        let (gen3, _) = core.debug_images().expect("thread").expect("images");
        assert_eq!(gen3, gen1, "running refetch of identical content skips too");
        core.pause().expect("thread");
    }

    #[serial_test::serial]
    #[test]
    fn images_store_advances_only_on_changed_content() {
        use nerust_core_traits::debugger::ImageFormat;
        let core = loaded_core();
        let picture = |pixels: Vec<u8>| {
            vec![DebugImage {
                id: "tiles",
                label_id: "Tiles",
                width: 2,
                height: 1,
                format: ImageFormat::Indexed2bpp,
                palette: vec![[0, 0, 0], [255, 255, 255]],
                pixels,
            }]
        };
        let first = core.images_store(picture(vec![0, 1]));
        assert!(first.fresh);
        let served = first.generation;
        // Identical content: same generation back, no freshness.
        let repeat = core.images_store(picture(vec![0, 1]));
        assert!(!repeat.fresh);
        assert_eq!(repeat.generation, served);
        // Changed byte: fresh with the current generation.
        core.mark_images_dirty();
        let changed = core.images_store(picture(vec![0, 2]));
        assert!(changed.fresh);
        assert!(changed.generation > served);
    }

    #[serial_test::serial]
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

    #[serial_test::serial]
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
