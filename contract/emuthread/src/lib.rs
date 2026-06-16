use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};

use nerust_contract_core::timer::FrameTimer;
use nerust_contract_core::{ConsoleCore, EmuCommand, GpuCommandList};

pub struct EmuThread<C: ConsoleCore + Send + 'static> {
    cmd_tx: Sender<EmuCommand>,
    last_cmds: Arc<RwLock<Option<GpuCommandList>>>,
    last_slot: Arc<RwLock<Vec<u8>>>,
    frame_count: Arc<AtomicU64>,
    render_pending: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _core: PhantomData<C>,
}

impl<C: ConsoleCore + Send + 'static> EmuThread<C> {
    pub fn spawn(mut core: C, timer: FrameTimer) -> Self {
        let (cmd_tx, cmd_rx): (Sender<EmuCommand>, Receiver<EmuCommand>) = mpsc::channel();
        let last_cmds: Arc<RwLock<Option<GpuCommandList>>> = Arc::new(RwLock::new(None));
        let last_slot: Arc<RwLock<Vec<u8>>> = Arc::new(RwLock::new(Vec::new()));
        let frame_count = Arc::new(AtomicU64::new(0));
        let render_pending = Arc::new(AtomicBool::new(false));

        let cmds = Arc::clone(&last_cmds);
        let slot = Arc::clone(&last_slot);
        let fc = Arc::clone(&frame_count);
        let rp = Arc::clone(&render_pending);
        let thread = thread::spawn(move || {
            let slot_size = core.frame_slot_size();
            let mut frame_slot = vec![0u8; slot_size];
            // 共有バッファを同じサイズに初期化（以降は swap でポインタ交換のみ）
            *slot.write().unwrap_or_else(|e| e.into_inner()) = vec![0u8; slot_size];
            let mut local_frame = 0u64;
            loop {
                match cmd_rx.recv() {
                    Ok(EmuCommand::RenderFrame) => {
                        match core.render_frame(&mut frame_slot) {
                            Ok(list) => {
                                *cmds.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
                                // ポインタ交換: frame_slot が新しいフレーム、
                                // shared が古いフレームになる（ゼロコピー）
                                let mut guard =
                                    slot.write().unwrap_or_else(|e| e.into_inner());
                                std::mem::swap(&mut *guard, &mut frame_slot);
                                rp.store(false, Ordering::Release);
                            }
                            Err(e) => {
                                log::error!("render_frame failed: {e}");
                            }
                        }
                        local_frame += 1;
                        fc.store(local_frame, Ordering::Relaxed);
                        if let Some(dur) = timer.remaining_until(local_frame) {
                            thread::sleep(dur);
                        }
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
            last_cmds,
            last_slot,
            frame_count,
            render_pending,
            thread: Some(thread),
            _core: PhantomData,
        }
    }

    pub fn send(&self, cmd: EmuCommand) -> Result<(), mpsc::SendError<EmuCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn request_frame(&self) {
        if !self.render_pending.swap(true, Ordering::Acquire) {
            let _ = self.cmd_tx.send(EmuCommand::RenderFrame);
        }
    }

    pub fn last_commands(&self) -> Option<GpuCommandList> {
        self.last_cmds
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn with_last_slot(&self, f: impl FnOnce(&[u8])) {
        if let Ok(guard) = self.last_slot.read() {
            if !guard.is_empty() {
                f(&guard);
            }
        }
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
