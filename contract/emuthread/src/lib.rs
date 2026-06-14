use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, GpuCommandList};
use nerust_timer::Timer;

/// `wait_frame()` の戻り値。GpuCommandList と描画済みスロットデータを一緒に返す。
#[derive(Clone)]
pub struct FrameResult {
    pub commands: GpuCommandList,
    pub slot_data: Arc<[u8]>,
}

pub struct EmuThread<C: ConsoleCore + Send + 'static> {
    cmd_tx: Sender<EmuCommand>,
    done_rx: Receiver<FrameResult>,
    last_fps: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
    _core: PhantomData<C>,
}

impl<C: ConsoleCore + Send + 'static> EmuThread<C> {
    pub fn spawn(mut core: C) -> Self {
        let frame_interval = core.frame_interval();
        let slot_size = 256 * 240 * 4;
        let (cmd_tx, cmd_rx): (Sender<EmuCommand>, Receiver<EmuCommand>) = mpsc::channel();
        let (done_tx, done_rx): (Sender<FrameResult>, Receiver<FrameResult>) = mpsc::channel();
        let last_fps = Arc::new(AtomicU32::new(0));

        let thread_fps = Arc::clone(&last_fps);
        let thread = thread::spawn(move || {
            let mut timer = Timer::new_with_interval(frame_interval);
            let mut slot = vec![0u8; slot_size];
            loop {
                match cmd_rx.recv() {
                    Ok(EmuCommand::RenderFrame) => {
                        let result = match core.render_frame(&mut slot) {
                            Ok(commands) => FrameResult {
                                commands,
                                slot_data: Arc::from(&slot[..]),
                            },
                            Err(e) => {
                                log::error!("render_frame failed: {e}");
                                FrameResult {
                                    commands: GpuCommandList {
                                        commands: vec![nerust_contract_core::GpuCommand::Blit {
                                            slot: 0,
                                        }],
                                    },
                                    slot_data: Arc::from(&slot[..]),
                                }
                            }
                        };
                        thread_fps.store((timer.as_fps() * 100.0) as u32, Ordering::Relaxed);
                        timer.wait();
                        let _ = done_tx.send(result);
                    }
                    Ok(EmuCommand::Pause) => core.set_paused(true),
                    Ok(EmuCommand::Resume) => core.set_paused(false),
                    Ok(EmuCommand::Reset) => core.reset(),
                    Ok(EmuCommand::SaveState(reply)) => {
                        let _ = reply.send(core.save_state());
                    }
                    Ok(EmuCommand::LoadState(data, reply)) => {
                        let _ = reply.send(core.load_state(&data));
                    }
                    Ok(EmuCommand::ApplyInputState(bytes)) => {
                        core.apply_input_state(&bytes);
                    }
                    Ok(EmuCommand::Quit) | Err(_) => break,
                }
            }
        });

        Self {
            cmd_tx,
            done_rx,
            last_fps,
            thread: Some(thread),
            _core: PhantomData,
        }
    }

    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::SendError<EmuCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn wait_frame(&self) -> Result<FrameResult, mpsc::RecvError> {
        self.done_rx.recv()
    }

    pub fn last_fps(&self) -> &AtomicU32 {
        &self.last_fps
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
