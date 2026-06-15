use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use nerust_contract_core::{ConsoleCore, EmuCommand, GpuCommandList};
use nerust_timer::Timer;

#[derive(Clone)]
pub struct FrameResult {
    pub commands: GpuCommandList,
    pub slot_data: Arc<[u8]>,
}

pub struct EmuThread<C: ConsoleCore + Send + 'static> {
    cmd_tx: Sender<EmuCommand>,
    last_result: Arc<Mutex<Option<FrameResult>>>,
    last_fps: Arc<AtomicU32>,
    frame_count: Arc<AtomicU64>,
    render_pending: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _core: PhantomData<C>,
}

impl<C: ConsoleCore + Send + 'static> EmuThread<C> {
    pub fn spawn(mut core: C) -> Self {
        let frame_interval = core.frame_interval();
        let slot_size = core.frame_slot_size();
        let (cmd_tx, cmd_rx) = mpsc::channel::<EmuCommand>();
        let last_fps = Arc::new(AtomicU32::new(0));
        let last_result: Arc<Mutex<Option<FrameResult>>> = Arc::new(Mutex::new(None));
        let render_pending = Arc::new(AtomicBool::new(false));
        let frame_count = Arc::new(AtomicU64::new(0));

        let thread_result = Arc::clone(&last_result);
        let thread_fps = Arc::clone(&last_fps);
        let thread_frames = Arc::clone(&frame_count);
        let thread_pending = Arc::clone(&render_pending);
        let thread = thread::spawn(move || {
            let mut timer = Timer::new_with_interval(frame_interval);
            let mut slot = vec![0u8; slot_size];
            loop {
                match cmd_rx.recv() {
                    Ok(EmuCommand::RenderFrame) => {
                        thread_pending.store(false, Ordering::Relaxed);
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
                        *thread_result.lock().unwrap() = Some(result);
                        thread_frames.fetch_add(1, Ordering::Relaxed);
                        thread_fps.store((timer.as_fps() * 100.0) as u32, Ordering::Relaxed);
                        timer.wait();
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
            last_result,
            last_fps,
            render_pending,
            frame_count,
            thread: Some(thread),
            _core: PhantomData,
        }
    }

    pub fn request_frame(&self) {
        if !self.render_pending.swap(true, Ordering::Relaxed) {
            let _ = self.cmd_tx.send(EmuCommand::RenderFrame);
        }
    }

    pub fn last_result(&self) -> Option<FrameResult> {
        self.last_result.lock().unwrap().clone()
    }

    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::SendError<EmuCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn last_fps(&self) -> &AtomicU32 {
        &self.last_fps
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
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
