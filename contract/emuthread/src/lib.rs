use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, GpuCommandList};

pub struct EmuThread<C: ConsoleCore + Send + 'static> {
    cmd_tx: Sender<EmuCommand>,
    done_rx: Receiver<()>,
    last_cmds: Arc<RwLock<Option<GpuCommandList>>>,
    thread: Option<JoinHandle<()>>,
    _core: PhantomData<C>,
}

impl<C: ConsoleCore + Send + 'static> EmuThread<C> {
    pub fn spawn(mut core: C) -> Self {
        let (cmd_tx, cmd_rx): (Sender<EmuCommand>, Receiver<EmuCommand>) = mpsc::channel();
        let (done_tx, done_rx): (Sender<()>, Receiver<()>) = mpsc::channel();
        let last_cmds: Arc<RwLock<Option<GpuCommandList>>> = Arc::new(RwLock::new(None));

        let cmds = Arc::clone(&last_cmds);
        let thread = thread::spawn(move || {
            let mut frame_slot = vec![0u8; core.frame_slot_size()];
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
                        let _ = done_tx.send(());
                    }
                    Ok(EmuCommand::Pause) => core.set_paused(true),
                    Ok(EmuCommand::Resume) => core.set_paused(false),
                    Ok(EmuCommand::Reset) => core.reset(),
                    Ok(EmuCommand::Quit) | Err(_) => break,
                }
            }
        });

        Self {
            cmd_tx,
            done_rx,
            last_cmds,
            thread: Some(thread),
            _core: PhantomData,
        }
    }

    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::SendError<EmuCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn wait_frame(&self) -> Result<(), mpsc::RecvError> {
        self.done_rx.recv()
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
