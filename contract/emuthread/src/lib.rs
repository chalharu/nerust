use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, FrameBuffer, GpuCommandList, PixelFormat};
use nerust_timer::Timer;

pub struct EmuThread {
    cmd_tx: SyncSender<EmuCommand>,
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
    /// `frame_ready` signals ConsoleVideo that a new frame is available.
    /// `palette` is the initial palette for the internal frame buffer (must match the renderer's palette).
    pub fn spawn(
        mut core: Box<dyn ConsoleCore + Send + 'static>,
        shared_fb: Arc<Mutex<FrameBuffer>>,
        frame_ready: Arc<AtomicBool>,
        palette: Box<[u32; 256]>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<EmuCommand>(8);
        let last_cmds: Arc<RwLock<Option<GpuCommandList>>> = Arc::new(RwLock::new(None));
        let frame_count: Arc<std::sync::atomic::AtomicU64> =
            Arc::new(std::sync::atomic::AtomicU64::new(0));
        let fps: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));

        let cmds = Arc::clone(&last_cmds);
        let fb = Arc::clone(&shared_fb);
        let fc = Arc::clone(&frame_count);
        let fps_c = Arc::clone(&fps);
        let fr = Arc::clone(&frame_ready);
        let thread = thread::spawn(move || {
            let mut frame_slot =
                FrameBuffer::with_capacity(256, 240, PixelFormat::PaletteIndex { palette });
            frame_slot.resize(256, 240);

            let mut timer = Timer::new();
            let mut loaded = false;
            loop {
                // When idle (no ROM loaded), block on recv() to avoid busy-looping.
                if !loaded {
                    match cmd_rx.recv() {
                        Ok(cmd) => match cmd {
                            EmuCommand::Load(cmd) => {
                                let result = core.load(&cmd.rom, &cmd.config);
                                loaded = result.is_ok();
                                let _ = cmd.reply.send(result);
                            }
                            EmuCommand::Quit => return,
                            _ => {}
                        },
                        Err(_) => return,
                    }
                    continue;
                }

                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        EmuCommand::Load(cmd) => {
                            let result = core.load(&cmd.rom, &cmd.config);
                            loaded = result.is_ok();
                            let _ = cmd.reply.send(result);
                        }
                        EmuCommand::Unload => {
                            core.unload();
                            loaded = false;
                        }
                        EmuCommand::Pause => core.set_paused(true),
                        EmuCommand::Resume => core.set_paused(false),
                        EmuCommand::Reset => core.reset(),
                        EmuCommand::SetVolume(vol) => core.set_volume(vol),
                        EmuCommand::SaveState { reply } => {
                            let result = core.save_state();
                            let _ = reply.send(result);
                        }
                        EmuCommand::LoadState(cmd) => {
                            let result = core.load_state(&cmd.data);
                            let _ = cmd.reply.send(result);
                        }
                        EmuCommand::MapperSave { reply } => {
                            let result = core.mapper_save();
                            let _ = reply.send(result);
                        }
                        EmuCommand::ImportMapperSave(cmd) => {
                            let result = core.import_mapper_save(&cmd.data);
                            let _ = cmd.reply.send(result);
                        }
                        EmuCommand::Identity { reply } => {
                            let result = core.identity();
                            let _ = reply.send(result);
                        }
                        EmuCommand::Quit => return,
                    }
                }

                if loaded && !core.paused() {
                    // render_frame only fails with NoRomLoaded (guarded by loaded flag)
                    if let Ok(list) = core.render_frame(&mut frame_slot) {
                        *cmds.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
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
            last_cmds,
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
