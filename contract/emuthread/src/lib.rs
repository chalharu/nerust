use std::fmt;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, FrameBuffer, GpuCommandList, PixelFormat};
use nerust_timer::Timer;

pub struct EmuThread<C: ConsoleCore + Send + 'static> {
    cmd_tx: Sender<EmuCommand>,
    done_rx: Receiver<()>,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    last_cmds: Arc<RwLock<Option<GpuCommandList>>>,
    thread: Option<JoinHandle<()>>,
    frame_count: Arc<std::sync::atomic::AtomicU64>,
    _core: PhantomData<C>,
}

impl<C: ConsoleCore + Send + 'static> fmt::Debug for EmuThread<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmuThread")
            .field("cmd_tx", &self.cmd_tx)
            .field("done_rx", &self.done_rx)
            .field("shared_fb", &self.shared_fb)
            .field("last_cmds", &self.last_cmds)
            .field("thread", &self.thread)
            .field("frame_count", &self.frame_count)
            .finish()
    }
}

impl<C: ConsoleCore + Send + 'static> EmuThread<C> {
    /// `shared_fb` is swapped with the internal frame buffer after each render_frame.
    pub fn spawn(mut core: C, shared_fb: Arc<Mutex<FrameBuffer>>) -> Self {
        let (cmd_tx, cmd_rx): (Sender<EmuCommand>, Receiver<EmuCommand>) = mpsc::channel();
        let (done_tx, done_rx): (Sender<()>, Receiver<()>) = mpsc::channel();
        let last_cmds: Arc<RwLock<Option<GpuCommandList>>> = Arc::new(RwLock::new(None));
        let frame_count: Arc<std::sync::atomic::AtomicU64> =
            Arc::new(std::sync::atomic::AtomicU64::new(0));

        let cmds = Arc::clone(&last_cmds);
        let fb = Arc::clone(&shared_fb);
        let fc = Arc::clone(&frame_count);
        let thread = thread::spawn(move || {
            let caps = core.capabilities();
            let default_format = caps
                .output_formats
                .first()
                .cloned()
                .unwrap_or(PixelFormat::PaletteIndex {
                    palette: Box::new([0u32; 256]),
                });
            let mut frame_slot = FrameBuffer::with_capacity(256, 240, default_format);
            frame_slot.resize(256, 240);

            let mut timer = Timer::new();

            'emu: loop {
                // Process all pending commands first
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        EmuCommand::Load { rom, config, reply } => {
                            let result = core.load(&rom, &config);
                            let _ = reply.send(result);
                        }
                        EmuCommand::Unload => core.unload(),
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
                        EmuCommand::RenderFrame => {} // auto-render loop makes this a no-op
                        EmuCommand::Quit => break 'emu,
                    }
                }

                // Auto-render one frame (if loaded and not paused)
                if !core.paused() {
                    match core.render_frame(&mut frame_slot) {
                        Ok(list) => {
                            *cmds.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
                            fc.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if let Ok(mut guard) = fb.lock() {
                                std::mem::swap(&mut *guard, &mut frame_slot);
                            }
                            let _ = done_tx.send(());
                        }
                        Err(e) => {
                            // NoRomLoaded is expected before any Load command
                            if !matches!(e, nerust_contract_core::CoreError::NoRomLoaded) {
                                log::error!("render_frame failed: {e}");
                            }
                        }
                    }
                }

                timer.wait();
            }
        });

        Self {
            cmd_tx,
            done_rx,
            shared_fb,
            last_cmds,
            thread: Some(thread),
            frame_count,
            _core: PhantomData,
        }
    }

    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::SendError<EmuCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn wait_frame(&self) -> Result<(), mpsc::RecvError> {
        self.done_rx.recv()
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
        self.frame_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.cmd_tx.send(EmuCommand::Quit);
            let _ = thread.join();
        }
    }
}

impl<C: ConsoleCore + Send + 'static> Drop for EmuThread<C> {
    fn drop(&mut self) {
        self.join();
    }
}
