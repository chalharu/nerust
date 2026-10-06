use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

use nerust_core_traits::{
    ConsoleCore, EmuCommand, LoadCommand,
    audio::{AudioBackend, StereoSample},
    debugger::{
        DUMP_ROW_BYTES, DebuggerError, HexRow, InspectError, InspectRequest, InspectResult,
        MAX_DUMP_BYTES, MemoryDump, StepUnit,
    },
};
use nerust_render_traits::FrameBuffer;
use nerust_sound_filter::dynamic_rate::DynamicRateFilter;
use nerust_timer::Timer;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleMetrics {
    pub frame_counter: u64,
    pub emulation_fps: f32,
    pub speed_multiplier: f32,
    pub loaded: bool,
    pub paused: bool,
}

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Reply(String),
}

pub struct EmuThread {
    cmd_tx: SyncSender<EmuCommand>,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    thread: Option<JoinHandle<()>>,
    frame_count: Arc<std::sync::atomic::AtomicU64>,
    fps: Arc<AtomicU32>,
}

impl fmt::Debug for EmuThread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmuThread")
            .field("cmd_tx", &self.cmd_tx)
            .field("shared_fb", &self.shared_fb)
            .field("thread", &self.thread)
            .field("frame_count", &self.frame_count)
            .field("fps", &self.fps.load(Ordering::Relaxed))
            .finish()
    }
}

impl EmuThread {
    /// `shared_fb` is swapped with the internal frame buffer after each render_frame.
    /// `frame_ready` signals ConsoleVideo that a new frame is available.
    /// `frame_slot` is the core's render target, built by the caller
    /// from the factory-declared profile (size + frame format) — never
    /// per-system constants here. `mem::swap` propagates its format
    /// into `shared_fb` and onward, so all three buffers must agree
    /// before the first swap; cores may still adjust it defensively.
    /// `audio` is session-owned: the thread pumps core-produced samples
    /// through the rate-control filter into it (transport separation:
    /// cores never see the backend).
    pub fn spawn(
        mut core: Box<dyn ConsoleCore + Send + 'static>,
        shared_fb: Arc<Mutex<FrameBuffer>>,
        frame_ready: Arc<AtomicBool>,
        frame_slot: FrameBuffer,
        mut audio: Box<dyn AudioBackend + Send + 'static>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<EmuCommand>(8);
        let frame_count: Arc<std::sync::atomic::AtomicU64> =
            Arc::new(std::sync::atomic::AtomicU64::new(0));
        let fps: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));

        let fb = Arc::clone(&shared_fb);
        let fc = Arc::clone(&frame_count);
        let fps_c = Arc::clone(&fps);
        let fr = Arc::clone(&frame_ready);
        let thread = thread::spawn(move || {
            let mut frame_slot = frame_slot;

            // Session-owned audio transport: core-produced nominal samples
            // collect here, then stretch through the rate-control filter
            // into the backend (both reused every frame, no allocation).
            let mut filter = DynamicRateFilter::new();
            let mut audio_scratch: Vec<nerust_core_traits::audio::StereoSample> = Vec::new();
            let mut nominal_tap: Option<Arc<Mutex<Vec<nerust_core_traits::audio::StereoSample>>>> =
                None;

            let mut timer = Timer::new();
            let mut loaded = false;
            loop {
                // When idle (no ROM loaded), block on recv() to avoid busy-looping.
                if !loaded {
                    match cmd_rx.recv() {
                        Ok(cmd) => match cmd {
                            EmuCommand::Load(cmd) => {
                                loaded = handle_load(&mut *core, &mut *audio, &mut filter, *cmd);
                            }
                            EmuCommand::Quit => return,
                            // Reply-bearing commands must answer even when
                            // idle: dropping the reply would hang the caller.
                            EmuCommand::Step { reply, .. } => {
                                let _ = reply.send(Err(DebuggerError::Unsupported));
                            }
                            EmuCommand::DebuggerInspect { reply, .. } => {
                                let _ =
                                    reply.send(Err(InspectError::Core(DebuggerError::Unsupported)));
                            }
                            EmuCommand::WriteMemory { reply, .. } => {
                                let _ = reply.send(Err(DebuggerError::Unsupported));
                            }
                            EmuCommand::TapNominalAudio { tap, reply } => {
                                nominal_tap = Some(tap);
                                let _ = reply.send(());
                            }
                            EmuCommand::DebuggerSpaces { reply, .. } => {
                                let _ = reply.send(Vec::new());
                            }
                            _ => {}
                        },
                        Err(_) => return,
                    }
                    continue;
                }

                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        EmuCommand::Load(cmd) => {
                            loaded = handle_load(&mut *core, &mut *audio, &mut filter, *cmd);
                        }
                        EmuCommand::Unload => {
                            core.unload();
                            loaded = false;
                            filter.reset();
                        }
                        EmuCommand::Pause => core.set_paused(true),
                        EmuCommand::Resume => core.set_paused(false),
                        EmuCommand::Reset => {
                            core.reset();
                            filter.reset();
                        }
                        EmuCommand::SetVolume(vol) => audio.set_volume(vol),
                        EmuCommand::RestartAudio => {
                            audio.reconnect();
                            filter.reset();
                        }
                        EmuCommand::SaveState { reply } => {
                            let result = core.save_state();
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::LoadState(cmd) => {
                            let result = core.load_state(&cmd.data);
                            filter.reset();
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = cmd.reply.send(result);
                        }
                        EmuCommand::MapperSave { reply } => {
                            let result = core.mapper_save();
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::ImportMapperSave(cmd) => {
                            let result = core.import_mapper_save(&cmd.data);
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = cmd.reply.send(result);
                        }
                        EmuCommand::Identity { reply } => {
                            let result = core.identity();
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::Step { unit, reply } => {
                            // Processed synchronously here, so receipt is
                            // the barrier: each reply matches exactly one
                            // executed step. Deterministic callers pause
                            // first; stepping while running races free-run.
                            //
                            // Frame steps run the standard frame block
                            // directly: the pixels must land in the shared
                            // framebuffer, which a delegated control
                            // cannot reach. The reply is the frame counter
                            // after the step (barrier position).
                            // Instruction steps carry no pixels, so they
                            // delegate to the control (cycles kept).
                            //
                            // Stepping an unloaded core is refused: the
                            // render block would silently succeed without
                            // advancing anything observable.
                            let result = if !loaded {
                                Err(DebuggerError::Unsupported)
                            } else {
                                match unit {
                                    StepUnit::Frame => {
                                        render_one_frame(FrameCtx {
                                            core: &mut *core,
                                            frame_slot: &mut frame_slot,
                                            audio_scratch: &mut audio_scratch,
                                            filter: &mut filter,
                                            audio: &mut *audio,
                                            nominal_tap: &nominal_tap,
                                            fb: &fb,
                                            fr: &fr,
                                            fc: &fc,
                                        });
                                        Ok(fc.load(Ordering::Relaxed))
                                    }
                                    StepUnit::Instruction => match core.debug_control() {
                                        Some(mut control) => control.step(unit),
                                        None => Err(DebuggerError::Unsupported),
                                    },
                                }
                            };
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::DebuggerInspect { req, reply } => {
                            let frame = fc.load(Ordering::Relaxed);
                            let result = inspect_memory(&*core, &req, frame);
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::WriteMemory { req, reply } => {
                            // Deliberately pause-ungated (see InspectError):
                            // deterministic callers pause first.
                            let result = match core.debug_control() {
                                Some(mut control) => {
                                    control.write_memory(req.space, req.addr, req.width, req.value)
                                }
                                None => Err(DebuggerError::Unsupported),
                            };
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::TapNominalAudio { tap, reply } => {
                            nominal_tap = Some(tap);
                            let _ = reply.send(());
                        }
                        EmuCommand::DebuggerSpaces { reply } => {
                            let result = match core.debugger() {
                                Some(debugger) => debugger.spaces().to_vec(),
                                None => Vec::new(),
                            };
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::Quit => return,
                    }
                }

                if loaded && !core.paused() {
                    // render_frame only fails with NoRomLoaded (guarded by loaded flag)
                    render_one_frame(FrameCtx {
                        core: &mut *core,
                        frame_slot: &mut frame_slot,
                        audio_scratch: &mut audio_scratch,
                        filter: &mut filter,
                        audio: &mut *audio,
                        nominal_tap: &nominal_tap,
                        fb: &fb,
                        fr: &fr,
                        fc: &fc,
                    });
                }

                timer.wait();
                fps_c.store(timer.as_fps().to_bits(), Ordering::Relaxed);
            }
        });

        Self {
            cmd_tx,
            shared_fb,
            thread: Some(thread),
            frame_count,
            fps,
        }
    }

    /// Send a command to the emu thread. Never blocks — uses `try_send`.
    /// Returns `Err(TrySendError)` if the channel is full or disconnected.
    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::TrySendError<EmuCommand>> {
        self.cmd_tx.try_send(cmd)
    }

    pub fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
        &self.shared_fb
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    pub fn fps(&self) -> f32 {
        f32::from_bits(self.fps.load(Ordering::Relaxed))
    }

    pub fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            // Quit send failure: thread already exited — expected during cleanup
            let _ = self.cmd_tx.send(EmuCommand::Quit);
            let _ = thread.join();
        }
    }
}

impl Drop for EmuThread {
    fn drop(&mut self) {
        self.join();
    }
}

/// Shared Load handling for the idle and running command loops:/// stamps the authoritative device rate (the core needs it as data —
/// resamplers, save-state validation — never the backend), loads the
/// ROM and restarts the rate filter. Returns whether a ROM is loaded.
/// Reply send failure (receiver dropped) is expected during teardown.
fn handle_load(
    core: &mut dyn ConsoleCore,
    audio: &mut dyn AudioBackend,
    filter: &mut DynamicRateFilter,
    mut cmd: LoadCommand,
) -> bool {
    cmd.config.audio_sample_rate = Some(audio.sample_rate());
    let result = core.load(&cmd.rom, &cmd.config);
    if result.is_ok() && cmd.start_paused {
        core.set_paused(true);
    }
    filter.reset();
    let loaded = result.is_ok();
    let _ = cmd.reply.send(result);
    loaded
}

/// Overwrite a nominal-audio tap with one frame's samples, when a tap
/// is installed and the frame produced any. The tap always holds the
/// latest frame only; accumulation is the reader's job.
fn push_nominal_tap(tap: &Option<Arc<Mutex<Vec<StereoSample>>>>, samples: Vec<StereoSample>) {
    if samples.is_empty() {
        return;
    }
    if let Some(tap) = tap
        && let Ok(mut guard) = tap.lock()
    {
        guard.clear();
        guard.extend_from_slice(&samples);
    }
}

/// Shared per-frame render state, threaded through the loop and the
/// `Step(Frame)` path so both render identically.
struct FrameCtx<'a> {
    core: &'a mut dyn ConsoleCore,
    frame_slot: &'a mut FrameBuffer,
    audio_scratch: &'a mut Vec<StereoSample>,
    filter: &'a mut DynamicRateFilter,
    audio: &'a mut dyn AudioBackend,
    nominal_tap: &'a Option<Arc<Mutex<Vec<StereoSample>>>>,
    fb: &'a Arc<Mutex<FrameBuffer>>,
    fr: &'a Arc<AtomicBool>,
    fc: &'a Arc<AtomicU64>,
}

/// Run one frame through the standard path: render, audio transport,
/// nominal tap, shared-buffer publish, and counter advance.
///
/// Free-run iterations and `Step(Frame)` share this so stepped frames
/// are indistinguishable from free ones.
fn render_one_frame(ctx: FrameCtx<'_>) {
    let FrameCtx {
        core,
        frame_slot,
        audio_scratch,
        filter,
        audio,
        nominal_tap,
        fb,
        fr,
        fc,
    } = ctx;
    // render_frame only fails with NoRomLoaded (callers guard loaded).
    if core.render_frame(frame_slot, audio_scratch).is_ok() {
        filter.push_frame(audio_scratch, audio);
        // Clone only with a tap installed: headless capture pays,
        // interactive frames do not.
        if nominal_tap.is_some() {
            push_nominal_tap(nominal_tap, audio_scratch.clone());
        }
        fc.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut guard) = fb.lock() {
            std::mem::swap(&mut *guard, frame_slot);
            fr.store(true, Ordering::Release);
        }
    }
}

/// Serve one inspect request against a paused core.
///
/// Pause is enforced here (infrastructure guard): reading while running
/// cannot guarantee the values match any single frame. Unmapped holes
/// inside the range end the dump; the final row may be short.
fn inspect_memory(
    core: &dyn ConsoleCore,
    req: &InspectRequest,
    frame: u64,
) -> Result<InspectResult, InspectError> {
    if !core.paused() {
        return Err(InspectError::NotPaused);
    }
    let debugger = core
        .debugger()
        .ok_or(InspectError::Core(DebuggerError::Unsupported))?;
    let (space, base) = match (req.space, req.addr) {
        (Some(id), Some(addr)) => (id, addr),
        // Documented default: start of the address map (viewer default view).
        _ => {
            let first = debugger
                .spaces()
                .first()
                .ok_or(InspectError::Core(DebuggerError::Unsupported))?;
            (first.id, *first.range.start())
        }
    };
    let max_rows = (req.rows as usize).min(MAX_DUMP_BYTES / DUMP_ROW_BYTES);
    let mut rows = Vec::with_capacity(max_rows);
    for row in 0..max_rows {
        let addr = base.saturating_add(row as u32 * DUMP_ROW_BYTES as u32);
        let mut bytes = [0u8; DUMP_ROW_BYTES];
        let n = debugger.read_bytes(space, addr, &mut bytes);
        if n == 0 {
            break;
        }
        rows.push(HexRow {
            addr,
            valid: n as u8,
            bytes,
        });
        if n < DUMP_ROW_BYTES {
            break;
        }
    }
    Ok(InspectResult {
        dump: MemoryDump {
            space,
            base,
            rows: rows.into(),
        },
        panels: debugger.panels().into(),
        registers: debugger.registers().to_vec().into(),
        captured_at_frame: frame,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::atomic::AtomicBool,
        sync::mpsc,
        sync::{Arc, Mutex},
    };

    use nerust_core_traits::{
        CoreConfig, CoreError,
        audio::{AudioBackend, StereoSample},
        debugger::{
            DebugControl, Debugger, DebuggerError, InspectError, InspectRequest, InspectResult,
            MemoryWrite, SpaceAccess, SpaceId, SpaceInfo, SpaceTable, StepUnit,
        },
    };

    use super::*;

    use nerust_render_traits::PixelFormat;

    struct SilentBackend;

    impl AudioBackend for SilentBackend {
        fn start(&mut self) {}
        fn pause(&mut self) {}
        fn push(&mut self, _sample: StereoSample) {}
        fn sample_rate(&self) -> u32 {
            48_000
        }
    }

    /// Minimal deterministic core: frames advance only via control.
    struct FakeCore {
        paused: bool,
        loaded: bool,
        stepped_frames: u64,
        mem: [u8; 256],
    }

    static FAKE_SPACES: [SpaceInfo; 1] = [SpaceInfo {
        id: SpaceId(0),
        key: "wram",
        name: "WRAM",
        address_bits: 8,
        range: 0x0000..=0x00FF,
        access: SpaceAccess::ReadWrite,
    }];

    static FAKE_TABLE: SpaceTable = SpaceTable::build(&FAKE_SPACES);

    struct FakeDebugger<'a> {
        mem: &'a [u8; 256],
        regs: Vec<(&'static str, u64)>,
    }

    impl<'a> FakeDebugger<'a> {
        fn new(mem: &'a [u8; 256]) -> Self {
            let mut regs = vec![("a", 1u64), ("b", 2u64)];
            regs.sort_by_key(|(name, _)| *name);
            Self { mem, regs }
        }

        fn read_byte(&self, addr: u32) -> Option<u64> {
            self.mem.get(addr as usize).copied().map(u64::from)
        }
    }

    impl Debugger for FakeDebugger<'_> {
        fn spaces(&self) -> &[SpaceInfo] {
            &FAKE_SPACES
        }

        fn space_containing(&self, addr: u32) -> Option<SpaceId> {
            (addr <= 0xFF).then_some(SpaceId(0))
        }

        fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
            if !FAKE_TABLE.covers(space, addr, width) {
                return None;
            }
            let mut value = 0u64;
            for i in 0..width as u32 {
                value |= self.read_byte(addr + i)? << (8 * i);
            }
            Some(value)
        }

        fn registers(&self) -> &[(&'static str, u64)] {
            &self.regs
        }
    }

    struct FakeControl<'a> {
        core: &'a mut FakeCore,
    }

    impl DebugControl for FakeControl<'_> {
        fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError> {
            match unit {
                StepUnit::Frame => {
                    self.core.stepped_frames += 1;
                    Ok(1_000)
                }
                StepUnit::Instruction => Err(DebuggerError::UnsupportedStepUnit(unit)),
            }
        }

        fn write_memory(
            &mut self,
            space: SpaceId,
            addr: u32,
            width: u8,
            value: u64,
        ) -> Result<(), DebuggerError> {
            if !SpaceTable::width_is_valid(width) {
                return Err(DebuggerError::BadWidth(width));
            }
            FAKE_TABLE
                .get(space)
                .ok_or(DebuggerError::UnknownSpace(space))?;
            if !FAKE_TABLE.covers(space, addr, width) {
                return Err(DebuggerError::UnmappedAddress { space, addr });
            }
            for i in 0..width as u32 {
                self.core.mem[(addr + i) as usize] = (value >> (8 * i)) as u8;
            }
            Ok(())
        }
    }

    impl ConsoleCore for FakeCore {
        fn capabilities(&self) -> nerust_core_traits::CoreCapabilities {
            nerust_core_traits::CoreCapabilities {
                output_formats: vec![],
                video_signal: nerust_core_traits::VideoSignalKind::Other,
            }
        }

        fn render_frame(
            &mut self,
            _frame_slot: &mut FrameBuffer,
            audio_out: &mut Vec<StereoSample>,
        ) -> Result<(), CoreError> {
            // Mirrors the console contract: clear, then produce one
            // deterministic sample. The tap observes exactly this stream.
            audio_out.clear();
            audio_out.push(StereoSample::new(2.0, 2.0));
            Ok(())
        }

        fn load(&mut self, _rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
            self.loaded = true;
            Ok(())
        }

        fn unload(&mut self) {
            self.loaded = false;
        }

        fn reset(&mut self) {}

        fn paused(&self) -> bool {
            self.paused
        }

        fn set_paused(&mut self, paused: bool) {
            self.paused = paused;
        }

        fn save_state(&self) -> Result<Vec<u8>, CoreError> {
            Ok(Vec::new())
        }

        fn load_state(&mut self, _data: &[u8]) -> Result<(), CoreError> {
            Ok(())
        }

        fn debug_control(
            &mut self,
        ) -> Option<Box<dyn nerust_core_traits::debugger::DebugControl + '_>> {
            // Mirrors real consoles: no ROM means no control handle.
            if !self.loaded {
                return None;
            }
            Some(Box::new(FakeControl { core: self }) as _)
        }

        fn debugger(&self) -> Option<Box<dyn Debugger + '_>> {
            if !self.loaded {
                return None;
            }
            Some(Box::new(FakeDebugger::new(&self.mem)) as _)
        }
    }

    fn spawn_loaded() -> EmuThread {
        let fb = Arc::new(Mutex::new(FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        )));
        let thread = EmuThread::spawn(
            Box::new(FakeCore {
                paused: false,
                loaded: false,
                stepped_frames: 0,
                mem: std::array::from_fn(|i| i as u8),
            }),
            Arc::clone(&fb),
            Arc::new(AtomicBool::new(false)),
            {
                let mut slot = FrameBuffer::with_capacity(
                    256,
                    240,
                    PixelFormat::PaletteIndex {
                        palette: Box::new([0u32; 256]),
                    },
                );
                slot.resize(256, 240);
                slot
            },
            Box::new(SilentBackend),
        );
        let (tx, rx) = mpsc::channel();
        thread
            .send(EmuCommand::Load(Box::new(LoadCommand {
                rom: vec![0u8; 16],
                config: CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                    core_options: None,
                    audio_sample_rate: None,
                },
                start_paused: false,
                reply: tx,
            })))
            .expect("load send");
        rx.recv().expect("load reply").expect("load ok");
        thread
    }

    fn step(thread: &EmuThread, unit: StepUnit) -> Result<u64, DebuggerError> {
        let (tx, rx) = mpsc::channel();
        thread
            .send(EmuCommand::Step { unit, reply: tx })
            .expect("step send");
        rx.recv().expect("step reply")
    }

    #[test]
    fn step_replies_are_synchronous_barriers() {
        let thread = spawn_loaded();
        thread.send(EmuCommand::Pause).expect("pause send");
        // Free-run frames may precede the pause; after it each reply
        // matches exactly one executed step, in order.
        let first = step(&thread, StepUnit::Frame).expect("step ok");
        let second = step(&thread, StepUnit::Frame).expect("step ok");
        assert_eq!(second, first + 1);
        assert_eq!(
            step(&thread, StepUnit::Instruction),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Instruction))
        );
    }

    #[test]
    fn paused_load_runs_no_free_frames() {
        let fb = Arc::new(Mutex::new(FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        )));
        let thread = EmuThread::spawn(
            Box::new(FakeCore {
                paused: false,
                loaded: false,
                stepped_frames: 0,
                mem: std::array::from_fn(|i| i as u8),
            }),
            Arc::clone(&fb),
            Arc::new(AtomicBool::new(false)),
            {
                let mut slot = FrameBuffer::with_capacity(
                    256,
                    240,
                    PixelFormat::PaletteIndex {
                        palette: Box::new([0u32; 256]),
                    },
                );
                slot.resize(256, 240);
                slot
            },
            Box::new(SilentBackend),
        );
        let (tx, rx) = mpsc::channel();
        thread
            .send(EmuCommand::Load(Box::new(LoadCommand {
                rom: vec![0u8; 16],
                config: CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                    core_options: None,
                    audio_sample_rate: None,
                },
                start_paused: true,
                reply: tx,
            })))
            .expect("load send");
        rx.recv().expect("load reply").expect("load ok");
        // Settle past several timer quanta: a free-running core would
        // have advanced; a paused one stays at zero.
        std::thread::sleep(std::time::Duration::from_millis(120));
        assert_eq!(thread.frame_count(), 0);
        // And stepping still works afterwards.
        assert!(step(&thread, StepUnit::Frame).is_ok());
        assert_eq!(thread.frame_count(), 1);
    }

    #[test]
    fn step_while_unloaded_is_unsupported() {
        let thread = spawn_loaded();
        thread.send(EmuCommand::Unload).expect("unload send");
        // Unload is processed before this Step (FIFO channel): the idle
        // arm must still answer instead of hanging the caller.
        assert_eq!(
            step(&thread, StepUnit::Frame),
            Err(DebuggerError::Unsupported)
        );
    }

    fn inspect(thread: &EmuThread, req: InspectRequest) -> Result<InspectResult, InspectError> {
        let (tx, rx) = mpsc::channel();
        thread
            .send(EmuCommand::DebuggerInspect { req, reply: tx })
            .expect("inspect send");
        rx.recv().expect("inspect reply")
    }

    fn write(thread: &EmuThread, req: MemoryWrite) -> Result<(), DebuggerError> {
        let (tx, rx) = mpsc::channel();
        thread
            .send(EmuCommand::WriteMemory { req, reply: tx })
            .expect("write send");
        rx.recv().expect("write reply")
    }

    fn paused_inspect_request() -> InspectRequest {
        InspectRequest {
            space: Some(SpaceId(0)),
            addr: Some(0x10),
            rows: 2,
        }
    }

    #[test]
    fn inspect_paused_reads_deterministic_bytes() {
        let thread = spawn_loaded();
        thread.send(EmuCommand::Pause).expect("pause send");
        let result = inspect(&thread, paused_inspect_request()).expect("inspect ok");
        assert_eq!(result.dump.space, SpaceId(0));
        assert_eq!(result.dump.base, 0x10);
        assert_eq!(result.dump.rows.len(), 2);
        // Fake memory holds its own address: row 0 covers 0x10..0x1F.
        let row = &result.dump.rows[0];
        assert_eq!(row.addr, 0x10);
        assert_eq!(row.valid, 16);
        assert_eq!(row.bytes[0], 0x10);
        assert_eq!(row.bytes[15], 0x1F);
        assert!(result.panels.is_empty());
    }

    #[test]
    fn inspect_while_running_is_refused() {
        let thread = spawn_loaded();
        // Never paused: the guard must refuse instead of racing free-run.
        assert!(matches!(
            inspect(&thread, paused_inspect_request()),
            Err(InspectError::NotPaused)
        ));
    }

    #[test]
    fn inspect_defaults_to_map_start() {
        let thread = spawn_loaded();
        thread.send(EmuCommand::Pause).expect("pause send");
        let result = inspect(
            &thread,
            InspectRequest {
                space: None,
                addr: None,
                rows: 1,
            },
        )
        .expect("inspect ok");
        assert_eq!(result.dump.space, SpaceId(0));
        assert_eq!(result.dump.base, 0);
        assert_eq!(result.dump.rows[0].bytes[0], 0);
    }

    #[test]
    fn write_roundtrip_reads_back_through_inspect() {
        let thread = spawn_loaded();
        thread.send(EmuCommand::Pause).expect("pause send");
        write(
            &thread,
            MemoryWrite {
                space: SpaceId(0),
                addr: 0x20,
                width: 1,
                value: 0xAB,
            },
        )
        .expect("write ok");
        let result = inspect(
            &thread,
            InspectRequest {
                space: Some(SpaceId(0)),
                addr: Some(0x20),
                rows: 1,
            },
        )
        .expect("inspect ok");
        assert_eq!(result.dump.rows[0].bytes[0], 0xAB);
        // Width validation precedes all other checks.
        assert_eq!(
            write(
                &thread,
                MemoryWrite {
                    space: SpaceId(9),
                    addr: 0,
                    width: 0,
                    value: 0,
                },
            ),
            Err(DebuggerError::BadWidth(0))
        );
    }

    #[test]
    fn nominal_tap_collects_stepped_frame_audio() {
        let thread = spawn_loaded();
        thread.send(EmuCommand::Pause).expect("pause send");
        let tap = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        thread
            .send(EmuCommand::TapNominalAudio {
                tap: Arc::clone(&tap),
                reply: tx,
            })
            .expect("tap send");
        rx.recv().expect("tap ack");
        step(&thread, StepUnit::Frame).expect("step ok");
        // The fake render emits one mono sample per frame; the tap holds
        // the latest stepped frame only.
        assert_eq!(
            *tap.lock().expect("tap lock"),
            vec![StereoSample::new(2.0, 2.0)]
        );
        // Take semantics: a second step overwrites, not appends.
        step(&thread, StepUnit::Frame).expect("step ok");
        assert_eq!(tap.lock().expect("tap lock").len(), 1);
    }
}
