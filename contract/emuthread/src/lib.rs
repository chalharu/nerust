use std::fmt;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, FrameBuffer, GpuCommandList, PixelFormat};

pub struct EmuThread<C: ConsoleCore + Send + 'static> {
    cmd_tx: Sender<EmuCommand>,
    done_rx: Receiver<()>,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    last_cmds: Arc<RwLock<Option<GpuCommandList>>>,
    thread: Option<JoinHandle<()>>,
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
            .finish()
    }
}

impl<C: ConsoleCore + Send + 'static> EmuThread<C> {
    /// `shared_fb` is swapped with the internal frame buffer after each render_frame.
    pub fn spawn(mut core: C, shared_fb: Arc<Mutex<FrameBuffer>>) -> Self {
        let (cmd_tx, cmd_rx): (Sender<EmuCommand>, Receiver<EmuCommand>) = mpsc::channel();
        let (done_tx, done_rx): (Sender<()>, Receiver<()>) = mpsc::channel();
        let last_cmds: Arc<RwLock<Option<GpuCommandList>>> = Arc::new(RwLock::new(None));

        let cmds = Arc::clone(&last_cmds);
        let fb = Arc::clone(&shared_fb);
        let thread = thread::spawn(move || {
            // Use core capabilities to size the initial FrameBuffer.
            // PaletteIndex is the NES default; other systems may use Rgba.
            let caps = core.capabilities();
            let default_format =
                caps.output_formats
                    .first()
                    .cloned()
                    .unwrap_or(PixelFormat::PaletteIndex {
                        palette: Box::new([0u32; 256]),
                    });
            let mut frame_slot = FrameBuffer::with_capacity(256, 240, default_format);
            loop {
                match cmd_rx.recv() {
                    Ok(EmuCommand::RenderFrame) => {
                        match core.render_frame(&mut frame_slot) {
                            Ok(list) => {
                                *cmds.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
                            }
                            Err(e) => {
                                log::error!("render_frame failed: {e}");
                            }
                        }
                        // swap internal fb with shared fb (zero-copy)
                        if let Ok(mut guard) = fb.lock() {
                            std::mem::swap(&mut *guard, &mut frame_slot);
                        }
                        let _ = done_tx.send(());
                    }
                    Ok(EmuCommand::Pause) => core.set_paused(true),
                    Ok(EmuCommand::Resume) => core.set_paused(false),
                    Ok(EmuCommand::Reset) => core.reset(),
                    Ok(EmuCommand::Load { rom, config, reply }) => {
                        let result = core.load(&rom, &config);
                        let _ = reply.send(result);
                    }
                    Ok(EmuCommand::Unload) => core.unload(),
                    Ok(EmuCommand::SetVolume(vol)) => core.set_volume(vol),
                    Ok(EmuCommand::SaveState { reply }) => {
                        let result = core.save_state();
                        let _ = reply.send(result);
                    }
                    Ok(EmuCommand::LoadState { data, reply }) => {
                        let result = core.load_state(&data);
                        let _ = reply.send(result);
                    }
                    Ok(EmuCommand::MapperSave { reply }) => {
                        let result = core.mapper_save();
                        let _ = reply.send(result);
                    }
                    Ok(EmuCommand::ImportMapperSave { data, reply }) => {
                        let result = core.import_mapper_save(&data);
                        let _ = reply.send(result);
                    }
                    Ok(EmuCommand::Identity { reply }) => {
                        let result = core.identity();
                        let _ = reply.send(result);
                    }
                    Ok(EmuCommand::Quit) | Err(_) => break,
                }
            }
        });

        Self {
            cmd_tx,
            done_rx,
            shared_fb,
            last_cmds,
            thread: Some(thread),
            _core: PhantomData,
        }
    }

    #[allow(clippy::result_large_err)]
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
