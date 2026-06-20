use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, FrameBuffer, GpuCommandList, PixelFormat};
use nerust_timer::Timer;

const MAX_CONSECUTIVE_ERRORS: u32 = 10;
const SUSPEND_RECOVERY_TICKS: u32 = 60; // ~1 second at 60fps

pub struct EmuThread {
    cmd_tx: Sender<EmuCommand>,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    last_cmds: Arc<RwLock<Option<GpuCommandList>>>,
    thread: Option<JoinHandle<()>>,
    frame_count: Arc<std::sync::atomic::AtomicU64>,
    fps: Arc<AtomicU32>,
}

impl fmt::Debug for EmuThread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmuThread")
            .field("cmd_tx", &self.cmd_tx)
            .field("shared_fb", &self.shared_fb)
            .field("last_cmds", &self.last_cmds)
            .field("thread", &self.thread)
            .field("frame_count", &self.frame_count)
            .field("fps", &self.fps.load(Ordering::Relaxed))
            .finish()
    }
}

impl EmuThread {
    /// `shared_fb` is swapped with the internal frame buffer after each render_frame.
    pub fn spawn(
        mut core: Box<dyn ConsoleCore + Send + 'static>,
        shared_fb: Arc<Mutex<FrameBuffer>>,
    ) -> Self {
        let (cmd_tx, cmd_rx): (Sender<EmuCommand>, Receiver<EmuCommand>) = mpsc::channel();
        let last_cmds: Arc<RwLock<Option<GpuCommandList>>> = Arc::new(RwLock::new(None));
        let frame_count: Arc<std::sync::atomic::AtomicU64> =
            Arc::new(std::sync::atomic::AtomicU64::new(0));
        let fps: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));

        let cmds = Arc::clone(&last_cmds);
        let fb = Arc::clone(&shared_fb);
        let fc = Arc::clone(&frame_count);
        let fps_c = Arc::clone(&fps);
        let thread = thread::spawn(move || {
            let caps = core.capabilities();
            let default_format =
                caps.output_formats
                    .first()
                    .cloned()
                    .unwrap_or(PixelFormat::PaletteIndex {
                        palette: Box::new([0u32; 256]),
                    });
            let mut frame_slot = FrameBuffer::with_capacity(256, 240, default_format);
            frame_slot.resize(256, 240);

            let mut timer = Timer::new();
            let mut loaded = false;
            let mut errors = 0u32;
            let mut suspend_ticks = 0u32;

            loop {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        EmuCommand::Load { rom, config, reply } => {
                            let result = core.load(&rom, &config);
                            loaded = result.is_ok();
                            errors = 0;
                            suspend_ticks = 0;
                            let _ = reply.send(result);
                        }
                        EmuCommand::Unload => {
                            core.unload();
                            loaded = false;
                            errors = 0;
                            suspend_ticks = 0;
                        }
                        EmuCommand::Pause => core.set_paused(true),
                        EmuCommand::Resume => core.set_paused(false),
                        EmuCommand::Reset => core.reset(),
                        EmuCommand::SetVolume(vol) => core.set_volume(vol),
                        EmuCommand::SaveState { reply } => {
                            let result = core.save_state();
                            let _ = reply.send(result);
                        }
                        EmuCommand::LoadState { data, reply } => {
                            let result = core.load_state(&data);
                            let _ = reply.send(result);
                        }
                        EmuCommand::MapperSave { reply } => {
                            let result = core.mapper_save();
                            let _ = reply.send(result);
                        }
                        EmuCommand::ImportMapperSave { data, reply } => {
                            let result = core.import_mapper_save(&data);
                            let _ = reply.send(result);
                        }
                        EmuCommand::Identity { reply } => {
                            let result = core.identity();
                            let _ = reply.send(result);
                        }
                        EmuCommand::Quit => return,
                    }
                }

                if loaded && !core.paused() && errors < MAX_CONSECUTIVE_ERRORS {
                    match core.render_frame(&mut frame_slot) {
                        Ok(list) => {
                            *cmds.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
                            fc.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut guard) = fb.lock() {
                                std::mem::swap(&mut *guard, &mut frame_slot);
                            }
                            errors = 0;
                        }
                        Err(e) => {
                            errors += 1;
                            log::error!(
                                "render_frame failed ({errors}/{MAX_CONSECUTIVE_ERRORS}): {e}"
                            );
                            if errors >= MAX_CONSECUTIVE_ERRORS {
                                suspend_ticks = SUSPEND_RECOVERY_TICKS;
                                log::error!(
                                    "emulation suspended for ~1s ({} ticks)",
                                    SUSPEND_RECOVERY_TICKS
                                );
                            }
                        }
                    }
                } else if suspend_ticks > 0 {
                    suspend_ticks -= 1;
                    if suspend_ticks == 0 {
                        errors = 0;
                        log::info!("emulation resumed after suspension period");
                    }
                }

                timer.wait();
                fps_c.store(timer.as_fps().to_bits(), Ordering::Relaxed);
            }
        });

        Self {
            cmd_tx,
            shared_fb,
            last_cmds,
            thread: Some(thread),
            frame_count,
            fps,
        }
    }

    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::SendError<EmuCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
        &self.shared_fb
    }

    pub fn last_commands(&self) -> Option<GpuCommandList> {
        self.last_cmds
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    pub fn fps(&self) -> f32 {
        f32::from_bits(self.fps.load(Ordering::Relaxed))
    }

    pub fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
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
