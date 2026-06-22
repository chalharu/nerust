use crate::load::MediaObject;
use crate::session::metrics::ConsoleMetrics;
use crate::session::persistence::CorePersistence;
use nerust_contract_core::identity::SystemIdentity;
use nerust_contract_core::{CoreConfig, EmuCommand, LoadCommand, StateDataCommand};
use nerust_contract_emuthread::EmuThread;
use nerust_screen_video::FrameBuffer;
use nerust_screen_video::VideoRenderProfile;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmuCoreError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Reply(String),
}

pub struct EmuCore {
    emu: EmuThread,
    render_profile: VideoRenderProfile,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    disp_fb: FrameBuffer,
    frame_ready: Arc<AtomicBool>,
    metrics: Arc<Mutex<ConsoleMetrics>>,
}

impl EmuCore {
    #[doc(hidden)]
    pub fn new(
        emu: EmuThread,
        render_profile: VideoRenderProfile,
        shared_fb: Arc<Mutex<FrameBuffer>>,
        disp_fb: FrameBuffer,
        frame_ready: Arc<AtomicBool>,
    ) -> Self {
        let metrics = Arc::new(Mutex::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        }));
        Self {
            emu,
            render_profile,
            shared_fb,
            disp_fb,
            frame_ready,
            metrics,
        }
    }

    pub fn render_profile(&self) -> &VideoRenderProfile {
        &self.render_profile
    }

    pub fn swap_frame_buffer(&mut self) {
        if self.frame_ready.load(Ordering::Relaxed)
            && let Ok(mut guard) = self.shared_fb.lock()
            && self.frame_ready.swap(false, Ordering::AcqRel)
        {
            std::mem::swap(&mut *guard, &mut self.disp_fb);
        }
        if let Ok(mut guard) = self.metrics.lock() {
            guard.frame_counter = self.emu.frame_count();
        } else {
            log::warn!("metrics lock poisoned in swap_frame_buffer");
        }
    }

    pub fn frame_buffer(&self) -> &FrameBuffer {
        &self.disp_fb
    }

    // metrics() のみ into_inner() で回復する理由: レンダリングパスで毎フレーム呼ばれ、
    // 常に最新の metrics を返す必要がある。他のメソッド(swap, pause, load 等)の metrics
    // 更新は副次的な副作用であり、poison 時に諦めても動作に影響しない。
    pub fn metrics(&self) -> ConsoleMetrics {
        let mut guard = self.metrics.lock().unwrap_or_else(|e| {
            log::warn!("metrics mutex poisoned, recovering");
            e.into_inner()
        });
        guard.frame_counter = self.emu.frame_count();
        guard.emulation_fps = self.emu.fps();
        *guard
    }

    pub fn set_volume(&self, volume: f32) -> Result<(), EmuCoreError> {
        self.emu
            .send(EmuCommand::SetVolume(volume))
            .map_err(|_| EmuCoreError::WorkerUnavailable)
    }

    pub fn resume(&self) -> Result<(), EmuCoreError> {
        self.emu
            .send(EmuCommand::Resume)
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = false,
            Err(e) => log::warn!("metrics lock poisoned in resume: {e}"),
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), EmuCoreError> {
        self.emu
            .send(EmuCommand::Pause)
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = true,
            Err(e) => log::warn!("metrics lock poisoned in pause: {e}"),
        }
        Ok(())
    }

    // TODO: CoreConfig に CoreOptions を統合する。blocked on NesConsoleCore::load
    // が &CoreConfig を受け取るように変更されること。
    pub fn load(&self, media: &MediaObject) -> Result<(), EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Load(Box::new(LoadCommand {
                rom: media.bytes.as_ref().to_vec(),
                config: CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                },
                reply: reply_tx,
            })))
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        let result = reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Reply(e.to_string()));
        if result.is_ok() {
            match self.metrics.lock() {
                Ok(mut guard) => {
                    guard.loaded = true;
                    guard.paused = true;
                }
                Err(e) => log::warn!("metrics lock poisoned in load: {e}"),
            }
        }
        result
    }

    pub fn unload(&self) -> Result<(), EmuCoreError> {
        self.emu
            .send(EmuCommand::Unload)
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.loaded = false,
            Err(e) => log::warn!("metrics lock poisoned in unload: {e}"),
        }
        Ok(())
    }

    pub fn reset(&self) -> Result<(), EmuCoreError> {
        self.emu
            .send(EmuCommand::Reset)
            .map_err(|_| EmuCoreError::WorkerUnavailable)
    }

    pub fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::MapperSave { reply: reply_tx })
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Reply(e.to_string()))
    }

    pub fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::ImportMapperSave(Box::new(StateDataCommand {
                data: bytes,
                reply: reply_tx,
            })))
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Reply(e.to_string()))
    }

    /// Raw state save: Send SaveState, return core bytes (no header, no preview).
    pub fn save_state_raw(&self) -> Result<Vec<u8>, EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SaveState { reply: reply_tx })
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Reply(e.to_string()))
    }

    /// Generate a preview frame from the EmuThread's shared frame buffer.
    pub fn generate_preview(&self) -> Option<crate::state::PreviewFrame> {
        crate::state::generate_preview(&self.emu)
    }

    pub fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Identity { reply: reply_tx })
            .map_err(|_| {
                log::warn!("canonical_media_identity: emu thread unavailable");
            })
            .ok()?;
        match reply_rx.recv() {
            Ok(Ok(identity)) => Some(identity),
            Ok(Err(e)) => {
                log::warn!("canonical_media_identity: core error: {e}");
                None
            }
            Err(_) => {
                log::warn!("canonical_media_identity: reply channel closed");
                None
            }
        }
    }

    /// Raw state load: Send LoadState with raw core bytes. No format fallback.
    pub fn load_state_raw(&self, data: Vec<u8>) -> Result<(), EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::LoadState(Box::new(StateDataCommand {
                data,
                reply: reply_tx,
            })))
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Reply(e.to_string()))
    }
}

impl CorePersistence for EmuCore {
    fn save_state_raw(&self) -> Result<Vec<u8>, EmuCoreError> {
        self.save_state_raw()
    }

    fn load_state_raw(&self, data: Vec<u8>) -> Result<(), EmuCoreError> {
        self.load_state_raw(data)
    }

    fn generate_preview(&self) -> Option<crate::state::PreviewFrame> {
        self.generate_preview()
    }

    fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        self.canonical_media_identity()
    }

    fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, EmuCoreError> {
        self.save_mapper_raw()
    }

    fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), EmuCoreError> {
        self.load_mapper_raw(bytes)
    }
}
