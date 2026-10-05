use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

use nerust_core_traits::{ConsoleCore, EmuCommand, LoadCommand, audio::AudioBackend};
use nerust_render_traits::{FrameBuffer, PixelFormat};
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
    /// `palette` is the initial palette for the internal frame buffer (must match the renderer's palette).
    /// `audio` is session-owned: the thread pumps core-produced samples
    /// through the rate-control filter into it (transport separation:
    /// cores never see the backend).
    pub fn spawn(
        mut core: Box<dyn ConsoleCore + Send + 'static>,
        shared_fb: Arc<Mutex<FrameBuffer>>,
        frame_ready: Arc<AtomicBool>,
        palette: Box<[u32]>,
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
            let mut frame_slot =
                FrameBuffer::with_capacity(256, 240, PixelFormat::PaletteIndex { palette });
            frame_slot.resize(256, 240);

            // Session-owned audio transport: core-produced nominal samples
            // collect here, then stretch through the rate-control filter
            // into the backend (both reused every frame, no allocation).
            let mut filter = DynamicRateFilter::new();
            let mut audio_scratch: Vec<nerust_core_traits::audio::StereoSample> = Vec::new();

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
                                let _ = reply.send(Err(
                                    nerust_core_traits::debugger::DebuggerError::Unsupported,
                                ));
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
                            let result = match core.debug_control() {
                                Some(mut control) => control.step(unit),
                                None => {
                                    Err(nerust_core_traits::debugger::DebuggerError::Unsupported)
                                }
                            };
                            // reply send failure: receiver dropped (timeout/abort) — expected
                            let _ = reply.send(result);
                        }
                        EmuCommand::Quit => return,
                    }
                }

                if loaded && !core.paused() {
                    // render_frame only fails with NoRomLoaded (guarded by loaded flag)
                    if core
                        .render_frame(&mut frame_slot, &mut audio_scratch)
                        .is_ok()
                    {
                        filter.push_frame(&audio_scratch, &mut *audio);
                        fc.fetch_add(1, Ordering::Relaxed);
                        if let Ok(mut guard) = fb.lock() {
                            std::mem::swap(&mut *guard, &mut frame_slot);
                            fr.store(true, Ordering::Release);
                        }
                    }
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

/// Shared Load handling for the idle and running command loops:
/// stamps the authoritative device rate (the core needs it as data —
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
    filter.reset();
    let loaded = result.is_ok();
    let _ = cmd.reply.send(result);
    loaded
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
        debugger::{DebugControl, DebuggerError, StepUnit},
    };

    use super::*;

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
            _space: nerust_core_traits::debugger::SpaceId,
            _addr: u32,
            _width: u8,
            _value: u64,
        ) -> Result<(), DebuggerError> {
            Err(DebuggerError::Unsupported)
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
            _audio_out: &mut Vec<StereoSample>,
        ) -> Result<(), CoreError> {
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
            }),
            Arc::clone(&fb),
            Arc::new(AtomicBool::new(false)),
            Box::new([0u32; 256]),
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
        // Each reply matches exactly one executed step, in order.
        assert_eq!(step(&thread, StepUnit::Frame), Ok(1_000));
        assert_eq!(step(&thread, StepUnit::Frame), Ok(1_000));
        assert_eq!(
            step(&thread, StepUnit::Instruction),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Instruction))
        );
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
}
