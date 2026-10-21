use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use crate::session::ports::StateExporter;
use nerust_core_traits::{
    CoreConfig, CoreOptions, EmuCommand, LoadCommand, StateDataCommand, SwapAudioCommand,
    SwapFrameSlotCommand,
    audio::AudioBackend,
    debugger::{
        DebugImage, DebuggerError, InspectError, InspectRequest, InspectResult, MemoryWrite,
        SnapshotRequest, SnapshotResult, SpaceInfo, StepUnit,
    },
    identity::SystemIdentity,
};
use nerust_emu_thread::{ConsoleMetrics, EmuThread, OperationError};
use nerust_input_traits::{AttachmentId, ControlId};
use nerust_render_traits::{FrameBuffer, PixelFormat, VideoFrameFormat, VideoRenderProfile};
use nerust_system_traits::factory::{CoreParts, load::MediaObject};

pub(crate) mod images;
mod pause_stable;

// Narrow live-core ports: playback, frame, audio, debug, persist. Single home for the persistence
// port; session depends down. Shell-owned preview frame: width plus packed RGBA bytes. The core names only this type; the persistence envelope adapts at the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRgba {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Packed RGBA bytes, `width * height * 4` long.
    pub rgba: Vec<u8>,
}

impl From<PreviewRgba> for crate::state::PreviewFrame {
    fn from(preview: PreviewRgba) -> Self {
        Self {
            width: preview.width,
            height: preview.height,
            rgba: preview.rgba,
        }
    }
}

impl From<crate::state::PreviewFrame> for PreviewRgba {
    fn from(frame: crate::state::PreviewFrame) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba,
        }
    }
}

/// Errors from core operations invoked by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum CorePersistenceError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Core(#[source] nerust_core_traits::CoreError),
}

impl CorePersistenceError {
    /// Typed fault for this error; single mapping point.
    pub fn fault(&self) -> StateFault {
        match self {
            Self::WorkerUnavailable | Self::NoReply => StateFault::Transport,
            Self::Core(error) => StateFault::from(error.fault()),
        }
    }

    /// Typed slot failure for this error; single mapping point.
    pub fn slot_failure(&self) -> SlotOpFailure {
        state_fault_to_failure(self.fault())
    }
}

/// Why a state-slot save/load did not execute.
/// Lives beside [`StateFault`]: the single mapping below is the only adapter in the shell.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SlotOpFailure {
    /// No active slot (nothing saved yet, or no core loaded).
    Empty,
    /// The slot file does not exist.
    Missing,
    /// Storage/backend I/O failure (filesystem, Android SAF, ...).
    Storage,
    /// The slot archive or its payload does not decode.
    Corrupt,
    /// The core rejected the payload (foreign ROM, options or schema
    /// mismatch, invalid state).
    Incompatible,
    /// Cannot operate (no states dir, no media identity, core export
    /// or thread failure).
    #[default]
    Unavailable,
}

/// Single mapping from core faults into slot failures; the only such adapter in the shell.
pub fn state_fault_to_failure(fault: StateFault) -> SlotOpFailure {
    use SlotOpFailure as F;
    match fault {
        StateFault::Transport => F::Unavailable,
        StateFault::Storage => F::Storage,
        StateFault::Missing => F::Missing,
        StateFault::Corrupt => F::Corrupt,
        StateFault::Incompatible => F::Incompatible,
        StateFault::Other(_) => F::Corrupt,
    }
}

/// Typed state fault end to end: transport vs storage vs undecodable vs rejected.
/// Single mapping point; no string sniffing at call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateFault {
    /// Thread gone or never answered.
    Transport,
    /// Backend I/O failure.
    Storage,
    /// Missing slot file.
    Missing,
    /// Undecodable payload.
    Corrupt,
    /// Core rejected the payload.
    Incompatible,
    /// Detail preserved for logs; never matched on.
    Other(String),
}

impl StateFault {
    /// Short code for logs and tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Storage => "storage",
            Self::Missing => "missing",
            Self::Corrupt => "corrupt",
            Self::Incompatible => "incompatible",
            Self::Other(_) => "other",
        }
    }
}

impl From<CorePersistenceError> for StateFault {
    fn from(error: CorePersistenceError) -> Self {
        error.fault()
    }
}

impl From<nerust_core_traits::CoreFault> for StateFault {
    fn from(fault: nerust_core_traits::CoreFault) -> Self {
        use nerust_core_traits::CoreFault;
        match fault {
            CoreFault::Unavailable => Self::Transport,
            CoreFault::Storage => Self::Storage,
            CoreFault::Missing => Self::Missing,
            CoreFault::Corrupt => Self::Corrupt,
            CoreFault::Incompatible => Self::Incompatible,
        }
    }
}

/// Shell-owned tick snapshot: one pass, no metrics lock.
/// Replaces the positional `(fps, paused, loaded, frames)` tuple.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickSnapshot {
    /// Emulation frames per second.
    pub fps: f32,
    /// Core pause flag.
    pub paused: bool,
    /// ROM loaded flag.
    pub loaded: bool,
    /// Thread frame counter.
    pub frames: u64,
}

/// Playback leg of the live core.
pub trait LivePlayback {
    fn pause(&self) -> Result<(), OperationError>;
    fn resume(&self) -> Result<(), OperationError>;
    fn reset(&self) -> Result<(), OperationError>;
    fn load(
        &self,
        media: &MediaObject,
        core_options: Option<Box<dyn CoreOptions>>,
    ) -> Result<(), OperationError>;
    fn unload(&self) -> Result<(), OperationError>;
    fn metrics(&self) -> ConsoleMetrics;
    fn tick_snapshot(&self) -> TickSnapshot;
    /// Lock-free pause flag: per-frame image gates read this instead of locking `metrics()`.
    fn is_paused(&self) -> bool {
        self.tick_snapshot().paused
    }
}

/// Frame leg of the live core.
pub trait LiveFramePort {
    fn render_profile(&self) -> &VideoRenderProfile;
    fn swap_frame_buffer(&mut self);
    fn frame_buffer(&self) -> &FrameBuffer;
    fn clear_display(&mut self);
    fn apply_presentation(
        &mut self,
        profile: VideoRenderProfile,
        palette: Box<[u32]>,
    ) -> Result<(), OperationError>;
}

/// Audio leg of the live core.
pub trait LiveAudioPort {
    fn set_volume(&self, volume: f32) -> Result<(), OperationError>;
    fn restart_audio(&self) -> Result<(), OperationError>;
    fn swap_audio_backend(
        &self,
        backend: Box<dyn AudioBackend>,
        sample_rate: u32,
    ) -> Result<(), OperationError>;
}

/// Narrow step leg of the debug port.
pub trait LiveStepPort {
    fn step(&self, unit: StepUnit) -> Result<Result<u64, DebuggerError>, OperationError>;
    /// Narrow send half for the async drain.
    fn step_send(
        &self,
        unit: StepUnit,
    ) -> Result<std::sync::mpsc::Receiver<Result<u64, DebuggerError>>, OperationError>;
}

/// Narrow inspect leg of the debug port.
pub trait LiveInspectPort {
    fn inspect(
        &self,
        req: InspectRequest,
    ) -> Result<Result<InspectResult, InspectError>, OperationError>;
    fn inspect_send(
        &self,
        req: InspectRequest,
    ) -> Result<std::sync::mpsc::Receiver<Result<InspectResult, InspectError>>, OperationError>;
}

/// Shared image batch: generation plus refcounted decode.
pub type SharedDebugImages = std::sync::Arc<Vec<DebugImage>>;
/// Debugger image outcome: thread errors outer, domain errors inner.
pub type DebugImagesOutcome =
    Result<Result<(u64, SharedDebugImages), DebuggerError>, OperationError>;

/// Narrow snapshot leg of the debug port.
pub trait LiveSnapshotPort {
    fn debug_snapshot_batch(
        &self,
        req: SnapshotRequest,
    ) -> Result<Result<SnapshotResult, DebuggerError>, OperationError>;
    fn debug_images(&self) -> DebugImagesOutcome;
    fn memory_spaces(&self) -> Result<Vec<SpaceInfo>, OperationError>;
    fn output_channels(&self) -> Result<Vec<String>, OperationError>;
    fn snapshot_batch_send(
        &self,
        req: SnapshotRequest,
    ) -> Result<std::sync::mpsc::Receiver<Result<SnapshotResult, DebuggerError>>, OperationError>;
}

/// Narrow memory-write leg of the debug port.
pub trait LiveMemoryPort {
    fn write_memory(&self, req: MemoryWrite) -> Result<Result<(), DebuggerError>, OperationError>;
    fn write_register(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<Result<(), DebuggerError>, OperationError>;
    fn write_memory_send(
        &self,
        req: MemoryWrite,
    ) -> Result<std::sync::mpsc::Receiver<Result<(), DebuggerError>>, OperationError>;
    fn write_register_send(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<std::sync::mpsc::Receiver<Result<(), DebuggerError>>, OperationError>;
}

/// Debug leg of the live core: union of the narrow step/inspect/snapshot/memory legs.
pub trait LiveDebugPort:
    LiveStepPort + LiveInspectPort + LiveSnapshotPort + LiveMemoryPort
{
}

impl<T: LiveStepPort + LiveInspectPort + LiveSnapshotPort + LiveMemoryPort + ?Sized> LiveDebugPort
    for T
{
}

/// Report from storing a fetched image decode: generation, pixels, and freshness.
#[derive(Debug)]
pub struct StoredImages {
    /// Source generation of the stored decode.
    pub generation: u64,
    /// Stored pixels.
    pub images: std::sync::Arc<Vec<DebugImage>>,
    /// Whether the content changed (viewers skip re-deriving when false).
    pub fresh: bool,
}

/// Narrow image-cache leg of the live core: paused PPU-viewer cache.
pub trait LiveImagePort {
    fn mark_images_dirty(&self);
    fn images_cache_lookup(&self) -> Option<(u64, std::sync::Arc<Vec<DebugImage>>)>;
    fn images_send(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<Result<Vec<DebugImage>, DebuggerError>>, OperationError>;
    fn images_store(&self, images: Vec<DebugImage>) -> StoredImages;
}

/// Full live core: all narrow legs plus the owned drain handle.
pub trait LiveCore:
    LivePlayback + LiveFramePort + LiveAudioPort + LiveDebugPort + LiveImagePort
{
    /// Owned handle for the async debug drain.
    fn drain_handle(&self) -> Box<dyn LiveCore>;
    /// Persistence view of this core; object-safe upcast for boxed holders.
    fn as_exporter(&self) -> &dyn StateExporter;
}

/// Cached system images for the debugger PPU viewer; viewers compare `generation`.
/// Anything advancing or replacing memory marks dirty.
#[derive(Debug, Clone)]
pub(crate) struct CachedImages {
    pub(crate) dirty: bool,
    pub(crate) generation: u64,
    pub(crate) images: std::sync::Arc<Vec<DebugImage>>,
    /// Content hash and generation of the last served decode, for the
    /// identical-content skip in [`EmuCore::images_store`].
    pub(crate) served_hash: Option<u64>,
    pub(crate) served_generation: u64,
}

/// FNV-1a content hash over decoded image bytes (dependency-free).
fn hash_debug_images(images: &[DebugImage]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    let mut mix = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for image in images {
        mix(image.id.as_bytes());
        mix(&image.width.to_le_bytes());
        mix(&image.height.to_le_bytes());
        for triple in &image.palette {
            mix(triple);
        }
        mix(&image.pixels);
    }
    hash
}

/// Bound for debugger query round-trips; interactive drains never wait on it.
/// It bounds only the blocking wrappers for tests, headless drivers, and non-drain callers.
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

/// Explicit shared-transport handle: same sharing as the old `Clone`, named.
impl EmuCore {
    /// Share transport, metrics, frame slot, and image cache; the framebuffer resets empty.
    pub(crate) fn shallow_handle(&self) -> Self {
        Self {
            emu: Arc::clone(&self.emu),
            render_profile: self.render_profile.clone(),
            shared_fb: Arc::clone(&self.shared_fb),
            disp_fb: FrameBuffer::with_capacity(0, 0, PixelFormat::Rgba),
            frame_ready: Arc::clone(&self.frame_ready),
            metrics: Arc::clone(&self.metrics),
            tick_paused: Arc::clone(&self.tick_paused),
            tick_loaded: Arc::clone(&self.tick_loaded),
            image_cache: Arc::clone(&self.image_cache),
        }
    }

    /// Boxed drain handle behind the [`LiveCore`] port.
    pub(crate) fn drain_handle_boxed(&self) -> Box<dyn LiveCore> {
        Box::new(self.shallow_handle())
    }
}

/// Parts returned when spawning a core: the core plus its input
/// adapter, field map, and host peripheral handles.
pub type CoreSpawnParts = (
    EmuCore,
    Box<dyn nerust_input_traits::GuiInputPort>,
    std::collections::HashMap<(AttachmentId, ControlId), usize>,
    nerust_core_traits::peripheral::HostPeripheralHandles,
);

pub struct EmuCore {
    emu: Arc<dyn EmuThreadPort>,
    render_profile: VideoRenderProfile,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    disp_fb: FrameBuffer,
    frame_ready: Arc<AtomicBool>,
    metrics: Arc<Mutex<ConsoleMetrics>>,
    /// Lock-free tick mirror: per-frame readers never touch the mutex.
    /// Every mutex write below also stamps these; `metrics()` refreshes them on read.
    tick_paused: Arc<AtomicBool>,
    tick_loaded: Arc<AtomicBool>,
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
            tick_paused: Arc::new(AtomicBool::new(true)),
            tick_loaded: Arc::new(AtomicBool::new(false)),
            image_cache: Arc::new(Mutex::new(CachedImages {
                dirty: true,
                generation: 0,
                images: Arc::new(Vec::new()),
                served_hash: None,
                served_generation: 0,
            })),
        }
    }

    /// Wrap `CoreParts` into an `EmuCore`. Buffer geometry comes from
    /// the factory render profile so all buffers agree pre-swap.
    pub fn from_parts(parts: CoreParts) -> CoreSpawnParts {
        let field_map = parts.field_map;
        use std::sync::Mutex;
        let new_buffer = || fresh_frame_buffer(&parts.render_profile, &parts.palette);
        let shared_fb = Arc::new(Mutex::new(new_buffer()));
        let disp_fb = new_buffer();
        let frame_slot = new_buffer();

        let frame_ready = Arc::new(AtomicBool::new(false));
        let emu = EmuThread::spawn(
            Box::new(pause_stable::PauseStableCore(parts.core)),
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
                tick_paused: Arc::new(AtomicBool::new(true)),
                tick_loaded: Arc::new(AtomicBool::new(false)),
                image_cache: Arc::new(Mutex::new(CachedImages {
                    dirty: true,
                    generation: 0,
                    images: Arc::new(Vec::new()),
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
            && let Ok(mut guard) = self.shared_fb.try_lock()
            && self.frame_ready.swap(false, Ordering::AcqRel)
        {
            std::mem::swap(&mut *guard, &mut self.disp_fb);
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

    // metrics() のみ into_inner() で回復する: 毎フレーム呼ばれ、常に最新の値を返す必要がある。
    // 他メソッドの metrics 更新は副次的副作用であり、poison 時に諦めても動作に影響しない。
    pub fn metrics(&self) -> ConsoleMetrics {
        let mut guard = self.metrics.lock().unwrap_or_else(|e| {
            log::warn!("metrics mutex poisoned, recovering");
            e.into_inner()
        });
        guard.frame_counter = self.emu.frame_count();
        guard.emulation_fps = self.emu.fps();
        self.stamp_tick(&guard);
        *guard
    }

    /// Stamp the lock-free tick mirror from one consistent snapshot.
    /// Every mutex write path calls this so per-frame readers stay lock-free.
    fn stamp_tick(&self, metrics: &ConsoleMetrics) {
        self.tick_paused.store(metrics.paused, Ordering::Relaxed);
        self.tick_loaded.store(metrics.loaded, Ordering::Relaxed);
    }

    /// Lock-free pause flag: stamped atomics, no mutex.
    pub fn is_paused(&self) -> bool {
        self.tick_paused.load(Ordering::Relaxed)
    }

    /// Lock-free tick snapshot for per-frame passes: pure read.
    pub fn tick_snapshot(&self) -> TickSnapshot {
        TickSnapshot {
            fps: self.emu.fps(),
            paused: self.tick_paused.load(Ordering::Relaxed),
            loaded: self.tick_loaded.load(Ordering::Relaxed),
            frames: self.emu.frame_count(),
        }
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

    /// Live presentation swap for filter-only changes, pausing around the reply barrier.
    /// No ROM reload, no state round-trip; dirties the debugger image cache.
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
            if was_running && self.resume().is_err() {
                // The pause already happened. This is not an unchanged queue rejection.
                return Err(OperationError::NoReply);
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
            // The frame slot and profile are already committed. Signal an uncertain/
            // partial compound outcome so callers compensate even if Resume was rejected.
            self.resume().map_err(|_| OperationError::NoReply)?;
        }
        Ok(())
    }

    /// Exchange the thread-owned render target behind a reply barrier.
    /// The caller swaps the session-side buffers only after this returns, while paused.
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
        self.mark_images_dirty();
        match self.metrics.lock() {
            Ok(mut guard) => {
                guard.paused = false;
                self.stamp_tick(&guard);
            }
            Err(e) => log::warn!("metrics lock poisoned in resume: {e}"),
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Pause)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        // Frames may have advanced since the last image fetch, even if no viewer polled.
        self.mark_images_dirty();
        match self.metrics.lock() {
            Ok(mut guard) => {
                guard.paused = true;
                self.stamp_tick(&guard);
            }
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

    /// Load and start paused: no free-run frame executes before the first command.
    /// Headless drivers use this for deterministic frame zero.
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
                // Stamp the requested state, not unconditionally paused: a forced stamp
                // would turn every load-then-resume into a bogus Resumed notice.
                Ok(mut guard) => {
                    guard.loaded = true;
                    guard.paused = start_paused;
                    self.stamp_tick(&guard);
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
            Ok(mut guard) => {
                guard.loaded = false;
                self.stamp_tick(&guard);
            }
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

    /// Preview frame from the shared buffer; `None` without a frame.
    fn generate_preview(&self) -> Option<PreviewRgba> {
        generate_preview_frame(&*self.emu)
    }

    /// Advance execution by one unit; the reply is the barrier: it arrives after exactly one step.
    /// Returns thread errors outer, domain step errors inner.
    pub fn step(&self, unit: StepUnit) -> Result<Result<u64, DebuggerError>, OperationError> {
        let reply_rx = self.step_send(unit)?;
        let result = recv_debug_reply(reply_rx);
        if matches!(result, Ok(Ok(_))) {
            self.mark_images_dirty();
        }
        result
    }

    /// Send half of [`EmuCore::step`]: the command queues without blocking for the caller tick.
    /// The async debug drain pairs this with `mark_images_dirty`, mirroring the blocking body.
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

    /// Batched debugger snapshot (inspect + anchor + disassembly) in one thread crossing.
    /// Same per-part contracts as the individual calls; prefer here when the whole set is needed.
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

    /// Send half of [`EmuCore::write_memory`]: queues without blocking for the caller tick.
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

    /// Install a guest-output tap for one channel: rendered frames move fresh bytes into `tap`.
    /// Headless capture only.
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

    /// Memory-space table snapshot through the emu thread; empty when idle or debugger-less.
    /// Static metadata: no pause gating required.
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
}

impl EmuCore {
    /// Mark cached system images stale; the next `debug_images` refetches and bumps generation.
    pub(crate) fn mark_images_dirty(&self) {
        if let Ok(mut cache) = self.image_cache.lock() {
            cache.dirty = true;
            cache.generation = cache.generation.wrapping_add(1);
        }
    }

    /// On-demand system images through the emu thread, cached while paused.
    /// Returns the source generation so viewers skip re-deriving known generations.
    pub fn debug_images(&self) -> DebugImagesOutcome {
        // A running core advances memory at any time: dirty first so the lookup below refetches.
        // Atomic pause flag: no metrics lock on the image path.
        if !self.is_paused() {
            self.mark_images_dirty();
        }
        if let Some((generation, shared)) = self.images_cache_lookup() {
            return Ok(Ok((generation, std::sync::Arc::clone(&shared))));
        }
        let result = recv_debug_reply(self.images_send()?);
        let images = match result {
            Ok(Ok(images)) => images,
            Ok(Err(error)) => return Ok(Err(error)),
            Err(_) => return Err(OperationError::NoReply),
        };
        let stored = self.images_store(images);
        Ok(Ok((stored.generation, stored.images)))
    }

    /// Paused-cache lookup for [`EmuCore::debug_images`]: a fresh hit skips the emu thread.
    /// Pure query with no side effects; callers mark dirty explicitly when memory may advance.
    pub(crate) fn images_cache_lookup(&self) -> Option<(u64, std::sync::Arc<Vec<DebugImage>>)> {
        self.image_cache
            .lock()
            .ok()
            .filter(|cache| !cache.dirty && !cache.images.is_empty())
            .map(|cache| (cache.generation, std::sync::Arc::clone(&cache.images)))
    }

    /// Send half of the [`EmuCore::debug_images`] fetch: queues without
    /// blocking so the async debug drain can poll on its own tick.
    pub(crate) fn images_send(
        &self,
    ) -> Result<mpsc::Receiver<Result<Vec<DebugImage>, DebuggerError>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerImages { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
    }

    /// Store a fetched decode and report freshness; a poisoned cache still serves.
    /// Content-identical refetches report the *served* generation so viewers skip re-deriving.
    pub(crate) fn images_store(&self, images: Vec<DebugImage>) -> StoredImages {
        let hash = hash_debug_images(&images);
        // One shared copy: the cache holds it, the report moves the input without re-cloning.
        let shared = std::sync::Arc::new(images);
        let mut cache = self.image_cache.lock().unwrap_or_else(|e| {
            log::warn!("image cache mutex poisoned, recovering");
            e.into_inner()
        });
        cache.images = std::sync::Arc::clone(&shared);
        cache.dirty = false;
        if cache.served_hash == Some(hash) {
            // Identical content collapses the dirty bump back to the served generation.
            // Same pixels, same generation: viewers skip re-deriving instead of flapping.
            cache.generation = cache.served_generation;
            StoredImages {
                generation: cache.served_generation,
                images: std::sync::Arc::clone(&shared),
                fresh: false,
            }
        } else {
            cache.served_hash = Some(hash);
            cache.served_generation = cache.generation;
            StoredImages {
                generation: cache.generation,
                images: std::sync::Arc::clone(&shared),
                fresh: true,
            }
        }
    }
}

impl LivePlayback for EmuCore {
    fn pause(&self) -> Result<(), OperationError> {
        self.pause()
    }

    fn resume(&self) -> Result<(), OperationError> {
        self.resume()
    }

    fn reset(&self) -> Result<(), OperationError> {
        self.reset()
    }

    fn load(
        &self,
        media: &MediaObject,
        core_options: Option<Box<dyn CoreOptions>>,
    ) -> Result<(), OperationError> {
        self.load(media, core_options)
    }

    fn unload(&self) -> Result<(), OperationError> {
        self.unload()
    }

    fn metrics(&self) -> ConsoleMetrics {
        self.metrics()
    }

    fn tick_snapshot(&self) -> TickSnapshot {
        self.tick_snapshot()
    }

    fn is_paused(&self) -> bool {
        self.is_paused()
    }
}

impl LiveFramePort for EmuCore {
    fn render_profile(&self) -> &VideoRenderProfile {
        self.render_profile()
    }

    fn swap_frame_buffer(&mut self) {
        self.swap_frame_buffer()
    }

    fn frame_buffer(&self) -> &FrameBuffer {
        self.frame_buffer()
    }

    fn clear_display(&mut self) {
        self.clear_display()
    }

    fn apply_presentation(
        &mut self,
        profile: VideoRenderProfile,
        palette: Box<[u32]>,
    ) -> Result<(), OperationError> {
        self.apply_presentation(profile, palette)
    }
}

impl LiveAudioPort for EmuCore {
    fn set_volume(&self, volume: f32) -> Result<(), OperationError> {
        self.set_volume(volume)
    }

    fn restart_audio(&self) -> Result<(), OperationError> {
        self.restart_audio()
    }

    fn swap_audio_backend(
        &self,
        backend: Box<dyn AudioBackend>,
        sample_rate: u32,
    ) -> Result<(), OperationError> {
        self.swap_audio_backend(backend, sample_rate)
    }
}

impl LiveStepPort for EmuCore {
    fn step(&self, unit: StepUnit) -> Result<Result<u64, DebuggerError>, OperationError> {
        self.step(unit)
    }

    fn step_send(
        &self,
        unit: StepUnit,
    ) -> Result<mpsc::Receiver<Result<u64, DebuggerError>>, OperationError> {
        self.step_send(unit)
    }
}

impl LiveInspectPort for EmuCore {
    fn inspect(
        &self,
        req: InspectRequest,
    ) -> Result<Result<InspectResult, InspectError>, OperationError> {
        self.inspect(req)
    }

    fn inspect_send(
        &self,
        req: InspectRequest,
    ) -> Result<mpsc::Receiver<Result<InspectResult, InspectError>>, OperationError> {
        self.inspect_send(req)
    }
}

impl LiveSnapshotPort for EmuCore {
    fn debug_snapshot_batch(
        &self,
        req: SnapshotRequest,
    ) -> Result<Result<SnapshotResult, DebuggerError>, OperationError> {
        self.debug_snapshot_batch(req)
    }

    fn debug_images(&self) -> DebugImagesOutcome {
        self.debug_images()
    }

    fn memory_spaces(&self) -> Result<Vec<SpaceInfo>, OperationError> {
        self.memory_spaces()
    }

    fn output_channels(&self) -> Result<Vec<String>, OperationError> {
        self.output_channels()
    }

    fn snapshot_batch_send(
        &self,
        req: SnapshotRequest,
    ) -> Result<mpsc::Receiver<Result<SnapshotResult, DebuggerError>>, OperationError> {
        self.snapshot_batch_send(req)
    }
}

impl LiveMemoryPort for EmuCore {
    fn write_memory(
        &self,
        req: nerust_core_traits::debugger::MemoryWrite,
    ) -> Result<Result<(), nerust_core_traits::debugger::DebuggerError>, OperationError> {
        self.write_memory(req)
    }

    fn write_register(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<Result<(), nerust_core_traits::debugger::DebuggerError>, OperationError> {
        self.write_register(name, value)
    }

    fn write_memory_send(
        &self,
        req: nerust_core_traits::debugger::MemoryWrite,
    ) -> Result<mpsc::Receiver<Result<(), DebuggerError>>, OperationError> {
        self.write_memory_send(req)
    }

    fn write_register_send(
        &self,
        name: &'static str,
        value: u64,
    ) -> Result<mpsc::Receiver<Result<(), DebuggerError>>, OperationError> {
        self.write_register_send(name, value)
    }
}

impl crate::emu_core::LiveImagePort for EmuCore {
    fn mark_images_dirty(&self) {
        self.mark_images_dirty()
    }

    fn images_cache_lookup(&self) -> Option<(u64, std::sync::Arc<Vec<DebugImage>>)> {
        self.images_cache_lookup()
    }

    fn images_send(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<Result<Vec<DebugImage>, DebuggerError>>, OperationError>
    {
        self.images_send()
    }

    fn images_store(&self, images: Vec<DebugImage>) -> StoredImages {
        self.images_store(images)
    }
}

impl LiveCore for EmuCore {
    fn drain_handle(&self) -> Box<dyn LiveCore> {
        self.drain_handle_boxed()
    }

    fn as_exporter(&self) -> &dyn StateExporter {
        self
    }
}

/// Domain-port converse: thread request, then the two stored mappings.
impl StateExporter for EmuCore {
    fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
        request_slot(|| {
            EmuThread::request_via(
                |cmd| self.emu.send(cmd),
                |reply| EmuCommand::SaveState { reply },
            )
        })
    }

    fn import_state(&self, data: Vec<u8>) -> Result<(), SlotOpFailure> {
        let result = request_slot(|| {
            EmuThread::request_via(
                |cmd| self.emu.send(cmd),
                |reply| EmuCommand::LoadState(Box::new(StateDataCommand { data, reply })),
            )
        });
        self.mark_images_dirty();
        result
    }

    fn export_preview(&self) -> Option<PreviewRgba> {
        self.generate_preview()
    }

    fn export_identity(&self) -> Option<SystemIdentity> {
        self.canonical_media_identity()
    }

    fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
        request_slot(|| {
            EmuThread::request_via(
                |cmd| self.emu.send(cmd),
                |reply| EmuCommand::MapperSave { reply },
            )
        })
    }

    fn import_mapper(&self, bytes: Vec<u8>) -> Result<(), SlotOpFailure> {
        let result = request_slot(|| {
            EmuThread::request_via(
                |cmd| self.emu.send(cmd),
                |reply| {
                    EmuCommand::ImportMapperSave(Box::new(StateDataCommand { data: bytes, reply }))
                },
            )
        });
        self.mark_images_dirty();
        result
    }
}

/// Thread request classified end to end: transport, then slot fault.
fn request_slot<T>(
    request: impl FnOnce() -> Result<T, OperationError<nerust_core_traits::CoreError>>,
) -> Result<T, SlotOpFailure> {
    request()
        .map_err(op_to_persistence)
        .map_err(|error| error.slot_failure())
}

fn op_to_persistence(e: OperationError<nerust_core_traits::CoreError>) -> CorePersistenceError {
    match e {
        OperationError::WorkerUnavailable => CorePersistenceError::WorkerUnavailable,
        OperationError::NoReply => CorePersistenceError::NoReply,
        OperationError::Reply(s) => CorePersistenceError::Core(s),
    }
}

/// Fresh render target for one profile + palette pair, shared by spawn and live swaps.
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
fn generate_preview_frame(emu: &dyn EmuThreadPort) -> Option<PreviewRgba> {
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
    Some(PreviewRgba {
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
        assert!(!core.metrics().paused);
        core
    }

    /// Scripted transport: answers one query, refuses the rest.
    /// Proves `EmuCore` programs against the port, never thread internals.
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

    #[test]
    fn resume_then_pause_invalidates_images_without_intermediate_fetch() {
        let core = loaded_core();
        core.pause().unwrap();
        core.images_store(vec![DebugImage {
            id: "tiles",
            label_id: "Tiles",
            width: 1,
            height: 1,
            format: nerust_core_traits::debugger::ImageFormat::Indexed2bpp,
            palette: vec![[0, 0, 0]],
            pixels: vec![0],
        }]);
        assert!(core.images_cache_lookup().is_some());
        core.resume().unwrap();
        core.pause().unwrap();
        assert!(core.images_cache_lookup().is_none());
    }

    #[test]
    fn presentation_resume_rejection_is_partial_not_unchanged() {
        #[derive(Debug)]
        struct RejectResume;
        impl EmuThreadPort for RejectResume {
            fn send(&self, command: EmuCommand) -> Result<(), mpsc::TrySendError<EmuCommand>> {
                match command {
                    EmuCommand::Pause => Ok(()),
                    EmuCommand::SwapFrameSlot(command) => {
                        command.reply.send(()).unwrap();
                        Ok(())
                    }
                    other => Err(mpsc::TrySendError::Disconnected(other)),
                }
            }
            fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
                panic!("unused")
            }
            fn frame_count(&self) -> u64 {
                0
            }
            fn fps(&self) -> f32 {
                0.0
            }
        }
        let profile = build_test_core_parts().render_profile;
        let frame = fresh_frame_buffer(&profile, &[0; 256]);
        let mut core = EmuCore::new(
            Arc::new(RejectResume),
            profile.clone(),
            Arc::new(Mutex::new(frame)),
            fresh_frame_buffer(&profile, &[0; 256]),
            Arc::new(AtomicBool::new(false)),
        );
        core.metrics.lock().unwrap().paused = false;
        let mut changed = profile;
        changed.logical_size.width = 602;
        assert!(matches!(
            core.apply_presentation(changed, vec![0; 256].into_boxed_slice()),
            Err(OperationError::NoReply)
        ));
        assert_eq!(core.render_profile().logical_size.width, 602);
        assert!(core.is_paused());
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
        // The mock decode never changes: refetches serve the served generation with no payload.
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

#[cfg(test)]
mod port_tests {
    use super::{LiveAudioPort, LiveFramePort, LivePlayback, LiveSnapshotPort};
    use crate::emu_core::*;
    use crate::test_helpers::build_test_core_parts;
    use nerust_system_traits::factory::load::MediaObject;

    fn loaded_core() -> EmuCore {
        let (core, _, _, _) = EmuCore::from_parts(build_test_core_parts());
        let media = MediaObject::new(None, vec![0u8; 16]);
        core.load(&media, None).expect("mock ROM loads");
        core
    }

    #[test]
    fn narrow_ports_delegate_to_core() {
        let core = loaded_core();
        LivePlayback::pause(&core).expect("pause");
        LivePlayback::resume(&core).expect("resume");
        LivePlayback::reset(&core).expect("reset");
        let _ = LivePlayback::metrics(&core);
        let snapshot = LivePlayback::tick_snapshot(&core);
        assert!(snapshot.loaded);
        assert!(!snapshot.paused);
        let _ = LiveFramePort::render_profile(&core);
        let _ = LiveFramePort::frame_buffer(&core);
        LiveAudioPort::set_volume(&core, 0.5).expect("volume");
        LiveAudioPort::restart_audio(&core).expect("restart");
        assert!(LiveSnapshotPort::memory_spaces(&core).is_ok());
        assert!(LiveSnapshotPort::output_channels(&core).is_ok());
        assert!(StateExporter::export_state(&core).is_ok());
        assert!(StateExporter::export_mapper(&core).is_ok());
    }

    #[test]
    fn shallow_handle_shares_transport_with_empty_framebuffer() {
        let core = loaded_core();
        // Old `Clone` vs new explicit handle: same sharing, named constructor.
        let handle = core.shallow_handle();
        assert_eq!(
            LivePlayback::tick_snapshot(&handle).frames,
            LivePlayback::tick_snapshot(&core).frames,
            "drain handle shares the frame counter"
        );
        assert_eq!(
            handle.frame_buffer().width(),
            0,
            "handle framebuffer stays empty"
        );
        // Boxed drain handle behind the port shares too.
        let boxed = LiveCore::drain_handle(&core);
        assert!(LivePlayback::metrics(&*boxed).loaded);
        assert!(boxed.as_exporter().export_identity().is_some());
    }

    #[test]
    fn is_paused_reads_atomic_without_metrics_lock() {
        let core = loaded_core();
        assert!(!LivePlayback::is_paused(&core));
        LivePlayback::pause(&core).expect("pause");
        assert!(LivePlayback::is_paused(&core));
        assert!(core.is_paused());
    }

    #[test]
    fn live_image_batch_names_tuple_fields() {
        use crate::emu_core::images::{LiveImage, LiveImageBatch};
        let image = LiveImage::from(("tiles".to_string(), 2u32, 1u32, vec![9u8]));
        assert_eq!(
            (image.label.as_str(), image.width, image.height),
            ("tiles", 2, 1)
        );
        assert_eq!(image.rgba, vec![9u8]);
        let batch = LiveImageBatch {
            generation: 7,
            images: vec![image],
        };
        assert_eq!(batch.generation, 7);
        assert_eq!(batch.images.len(), 1);
    }

    #[test]
    fn tick_snapshot_reads_without_writing() {
        let core = loaded_core();
        let first = core.tick_snapshot();
        let second = core.tick_snapshot();
        assert_eq!((first.paused, first.loaded), (second.paused, second.loaded));
    }

    #[test]
    fn exporter_round_trips_state_and_mapper() {
        use crate::session::ports::StateExporter;
        let core = loaded_core();
        let saved = core.export_state().expect("save");
        core.import_state(saved).expect("load");
        let mapper = core.export_mapper().expect("mapper save");
        if let Some(bytes) = mapper {
            core.import_mapper(bytes).expect("mapper load");
        }
        assert!(core.export_identity().is_some());
        let _ = core.export_preview();
        let _ = LiveCore::drain_handle(&core);
        fn bound<T: StateExporter>() {}
        bound::<EmuCore>();
    }
}

#[cfg(test)]
mod narrow_tests {
    use super::*;
    use super::{LiveAudioPort, LiveFramePort, LivePlayback};
    use crate::session::ports::StateExporter;

    /// Static bound check: the core implements every narrow leg plus the domain port.
    #[test]
    fn core_implements_all_narrow_ports() {
        fn bound<
            T: LivePlayback
                + LiveFramePort
                + LiveAudioPort
                + LiveDebugPort
                + LiveImagePort
                + StateExporter,
        >() {
        }
        bound::<EmuCore>();
    }
}

#[cfg(test)]
mod fault_tests {
    use crate::emu_core::*;

    #[test]
    fn fault_classification_uses_typed_source_not_diagnostic_words() {
        use nerust_core_traits::{CoreError, CoreFault};
        for (kind, expected) in [
            (CoreFault::Storage, StateFault::Storage),
            (CoreFault::Missing, StateFault::Missing),
            (CoreFault::Corrupt, StateFault::Corrupt),
            (CoreFault::Incompatible, StateFault::Incompatible),
            (CoreFault::Unavailable, StateFault::Transport),
        ] {
            let error = CorePersistenceError::Core(CoreError::Persistence {
                kind,
                source: "foreign missing storage version mismatch".into(),
            });
            assert_eq!(error.fault(), expected);
        }
        assert_eq!(
            StateFault::from(CorePersistenceError::WorkerUnavailable),
            StateFault::Transport
        );
        assert_eq!(
            StateFault::from(CorePersistenceError::NoReply),
            StateFault::Transport
        );
    }

    #[test]
    fn fault_covers_codes_and_remaining_branches() {
        for (fault, code) in [
            (StateFault::Transport, "transport"),
            (StateFault::Storage, "storage"),
            (StateFault::Missing, "missing"),
            (StateFault::Corrupt, "corrupt"),
            (StateFault::Incompatible, "incompatible"),
            (StateFault::Other("x".into()), "other"),
        ] {
            assert_eq!(fault.code(), code);
        }
        assert_eq!(
            StateFault::from(CorePersistenceError::NoReply),
            StateFault::Transport
        );
        assert_eq!(
            StateFault::from(CorePersistenceError::Core(
                nerust_core_traits::CoreError::Core("gone".into())
            )),
            StateFault::Corrupt
        );
        assert_eq!(
            CorePersistenceError::WorkerUnavailable.fault(),
            StateFault::Transport
        );
        assert_eq!(CorePersistenceError::NoReply.fault(), StateFault::Transport);
        assert_eq!(
            CorePersistenceError::Core(nerust_core_traits::CoreError::Core("absent".into()))
                .fault(),
            StateFault::Corrupt
        );
        assert!(!CorePersistenceError::NoReply.to_string().is_empty());
    }

    #[test]
    fn slot_failure_maps_every_arm_once() {
        use SlotOpFailure as F;
        for (fault, failure) in [
            (StateFault::Transport, F::Unavailable),
            (StateFault::Storage, F::Storage),
            (StateFault::Missing, F::Missing),
            (StateFault::Corrupt, F::Corrupt),
            (StateFault::Incompatible, F::Incompatible),
            (StateFault::Other("x".into()), F::Corrupt),
        ] {
            assert_eq!(state_fault_to_failure(fault), failure);
        }
    }

    #[test]
    fn preview_round_trips_through_state_frame() {
        let preview = PreviewRgba {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let frame: crate::state::PreviewFrame = preview.clone().into();
        let back: PreviewRgba = frame.into();
        assert_eq!(back, preview);
    }

    #[test]
    fn fresh_frame_buffer_covers_palette_and_rgba() {
        let base = crate::test_helpers::build_test_core_parts().render_profile;
        let palette: Vec<u32> = (0..256).map(|i| 0xFF00_0000 | i).collect();
        let pal = fresh_frame_buffer(
            &VideoRenderProfile {
                frame_format: VideoFrameFormat::Palette,
                ..base.clone()
            },
            &palette,
        );
        assert!(pal.palette().is_some());
        let rgba = fresh_frame_buffer(
            &VideoRenderProfile {
                frame_format: VideoFrameFormat::Rgba,
                ..base
            },
            &palette,
        );
        assert!(rgba.palette().is_none());
    }

    #[test]
    fn preview_returns_none_without_frame() {
        #[derive(Debug)]
        struct EmptyPort;
        impl EmuThreadPort for EmptyPort {
            fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::TrySendError<EmuCommand>> {
                Err(mpsc::TrySendError::Disconnected(cmd))
            }
            fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
                static CELL: std::sync::OnceLock<Arc<Mutex<FrameBuffer>>> =
                    std::sync::OnceLock::new();
                CELL.get_or_init(|| {
                    let mut fb = FrameBuffer::with_capacity(0, 0, PixelFormat::Rgba);
                    fb.resize(0, 0);
                    Arc::new(Mutex::new(fb))
                })
            }
            fn frame_count(&self) -> u64 {
                0
            }
            fn fps(&self) -> f32 {
                0.0
            }
        }
        assert!(generate_preview_frame(&EmptyPort).is_none());
        let (tx, rx) = mpsc::channel::<u8>();
        drop(tx);
        assert!(matches!(recv_debug_reply(rx), Err(OperationError::NoReply)));
    }
}
