use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use nerust_core_traits::factory::{CoreParts, load::MediaObject};
use nerust_core_traits::{
    CoreConfig, EmuCommand, LoadCommand, StateDataCommand, identity::SystemIdentity,
};
use nerust_emu_thread::{ConsoleMetrics, EmuThread, OperationError};
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput};
use nerust_render_base::{FrameBuffer, PixelFormat, VideoRenderProfile};

use crate::session::persistence::{CorePersistence, CorePersistenceError};

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

    /// Wrap `CoreParts` (from a factory) into an `EmuCore`.
    /// Returns (EmuCore, GuiInput, field_map).
    pub fn from_parts(
        parts: CoreParts,
    ) -> (
        Self,
        GuiInput,
        std::collections::HashMap<(AttachmentId, DigitalControlId), usize>,
    ) {
        let field_map = parts.field_map;
        use std::sync::Mutex;
        let src_w = parts.render_profile.source_logical_size.width;
        let src_h = parts.render_profile.source_logical_size.height;
        let pixel_format = PixelFormat::PaletteIndex {
            palette: parts.palette.clone(),
        };

        let shared_fb = Arc::new(Mutex::new(FrameBuffer::with_capacity(
            src_w,
            src_h,
            pixel_format.clone(),
        )));
        if let Ok(mut guard) = shared_fb.lock() {
            guard.resize(src_w, src_h);
            guard.resize_data(src_w * src_h);
        }

        let mut disp_fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format);
        disp_fb.resize(src_w, src_h);
        disp_fb.resize_data(src_w * src_h);

        let frame_ready = Arc::new(AtomicBool::new(false));
        let emu = EmuThread::spawn(
            parts.core,
            Arc::clone(&shared_fb),
            Arc::clone(&frame_ready),
            parts.palette,
        );
        (
            Self {
                emu,
                render_profile: parts.render_profile,
                shared_fb,
                disp_fb,
                frame_ready,
                metrics: Arc::new(Mutex::new(ConsoleMetrics {
                    paused: true,
                    ..ConsoleMetrics::default()
                })),
            },
            parts.gui_input,
            field_map,
        )
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

    pub fn clear_display(&mut self) {
        self.disp_fb.clear();
        if let Ok(mut guard) = self.shared_fb.lock() {
            guard.clear();
        }
        self.frame_ready.store(false, Ordering::Release);
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

    pub fn set_volume(&self, volume: f32) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::SetVolume(volume))
            .map_err(|_| OperationError::WorkerUnavailable)
    }

    pub fn resume(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Resume)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = false,
            Err(e) => log::warn!("metrics lock poisoned in resume: {e}"),
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Pause)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = true,
            Err(e) => log::warn!("metrics lock poisoned in pause: {e}"),
        }
        Ok(())
    }

    pub fn load(&self, media: &MediaObject, core_options: Vec<u8>) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Load(Box::new(LoadCommand {
                rom: media.bytes.as_ref().to_vec(),
                config: CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                    core_options,
                },
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        let result = reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()));
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

    pub fn unload(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Unload)
            .map_err(|_| OperationError::WorkerUnavailable)?;
        match self.metrics.lock() {
            Ok(mut guard) => guard.loaded = false,
            Err(e) => log::warn!("metrics lock poisoned in unload: {e}"),
        }
        Ok(())
    }

    pub fn reset(&self) -> Result<(), OperationError> {
        self.emu
            .send(EmuCommand::Reset)
            .map_err(|_| OperationError::WorkerUnavailable)
    }

    pub fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::MapperSave { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()))
    }

    pub fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::ImportMapperSave(Box::new(StateDataCommand {
                data: bytes,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()))
    }

    /// Raw state save: Send SaveState, return core bytes (no header, no preview).
    pub fn save_state_raw(&self) -> Result<Vec<u8>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SaveState { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()))
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
    pub fn load_state_raw(&self, data: Vec<u8>) -> Result<(), OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::LoadState(Box::new(StateDataCommand {
                data,
                reply: reply_tx,
            })))
            .map_err(|_| OperationError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| OperationError::NoReply)?
            .map_err(|e| OperationError::Reply(e.to_string()))
    }
}

impl CorePersistence for EmuCore {
    fn save_state_raw(&self) -> Result<Vec<u8>, CorePersistenceError> {
        self.save_state_raw().map_err(op_to_persistence)
    }

    fn load_state_raw(&self, data: Vec<u8>) -> Result<(), CorePersistenceError> {
        self.load_state_raw(data).map_err(op_to_persistence)
    }

    fn generate_preview(&self) -> Option<crate::state::PreviewFrame> {
        self.generate_preview()
    }

    fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        self.canonical_media_identity()
    }

    fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, CorePersistenceError> {
        self.save_mapper_raw().map_err(op_to_persistence)
    }

    fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), CorePersistenceError> {
        self.load_mapper_raw(bytes).map_err(op_to_persistence)
    }
}

fn op_to_persistence(e: OperationError) -> CorePersistenceError {
    match e {
        OperationError::WorkerUnavailable => CorePersistenceError::WorkerUnavailable,
        OperationError::NoReply => CorePersistenceError::NoReply,
        OperationError::Reply(s) => CorePersistenceError::Core(s),
    }
}
