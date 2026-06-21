use crate::load::{MediaObject, ResolvedLoadRequest};
use crate::session::metrics::ConsoleMetrics;
use crate::state::{ConsoleStatePayload, PreviewFrame, RuntimeStateExport};

use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_contract_core::{
    CoreConfig, EmuCommand, LoadCommand, StateDataCommand, load_state_from_header,
    save_state_with_header,
};
use nerust_contract_emuthread::EmuThread;
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_nes_core::controller::Controller;
use nerust_screen_video::FilterType;
use nerust_screen_video::LogicalSize;
use nerust_screen_video::VideoRenderProfile;
use nerust_screen_video::{FrameBuffer, PixelFormat, VideoFrameHandle};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum EmuCoreError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Core(String),
}

#[derive(Debug, Clone)]
pub struct SystemRuntimeSnapshot {
    pub metrics: ConsoleMetrics,
    pub video_frame: Option<VideoFrameHandle>,
}

pub(crate) struct EmuCore {
    emu: EmuThread,
    render_profile: VideoRenderProfile,
    shared_fb: Arc<Mutex<FrameBuffer>>,
    disp_fb: FrameBuffer,
    frame_ready: Arc<AtomicBool>,
    metrics: Arc<Mutex<ConsoleMetrics>>,
}

impl EmuCore {
    pub fn new_gpu(
        speaker: Box<dyn AudioBackend + Send>,
        filter_type: FilterType,
        source_logical_size: LogicalSize,
        controller: Box<dyn Controller + Send>,
    ) -> Self {
        let layout = filter_type.layout(source_logical_size);
        let assets = filter_type.palette_console_video_assets();
        let ntsc_packed_rgba8 = assets
            .packed_ntsc_rgba8()
            .map(|data| data.to_vec().into_boxed_slice());
        let render_profile = VideoRenderProfile {
            source_logical_size: layout.source_logical_size,
            logical_size: layout.logical_size,
            physical_size: layout.physical_size,
            frame_format: nerust_screen_video::VideoFrameFormat::Palette,
            ntsc_packed_rgba8,
        };
        let mut palette = [0u32; 256];
        let rgba8 = assets.palette_rgba8();
        for (i, entry) in palette.iter_mut().enumerate().take(64) {
            let pos = i * 4;
            *entry = u32::from(rgba8[pos]) << 24
                | u32::from(rgba8[pos + 1]) << 16
                | u32::from(rgba8[pos + 2]) << 8
                | u32::from(rgba8[pos + 3]);
        }
        let pixel_format = PixelFormat::PaletteIndex {
            palette: Box::new(palette),
        };
        let src_w = source_logical_size.width;
        let src_h = source_logical_size.height;

        Self::build(
            speaker,
            render_profile,
            pixel_format,
            src_w,
            src_h,
            controller,
        )
    }

    fn build(
        speaker: Box<dyn AudioBackend + Send>,
        render_profile: VideoRenderProfile,
        pixel_format: PixelFormat,
        src_w: usize,
        src_h: usize,
        controller: Box<dyn Controller + Send>,
    ) -> Self {
        let shared_fb = Arc::new(Mutex::new(FrameBuffer::with_capacity(
            src_w,
            src_h,
            pixel_format.clone(),
        )));
        if let Ok(mut guard) = shared_fb.lock() {
            guard.resize(src_w, src_h);
            guard.resize_data(src_w * src_h);
        }

        let mut disp_fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format.clone());
        disp_fb.resize(src_w, src_h);
        disp_fb.resize_data(src_w * src_h);

        let metrics = Arc::new(Mutex::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        }));

        let mut speaker = speaker;
        speaker.start();
        let core = NesConsoleCore::new_empty(controller, speaker);
        let frame_ready = Arc::new(AtomicBool::new(false));
        let palette = match &pixel_format {
            PixelFormat::PaletteIndex { palette } => palette.clone(),
            PixelFormat::Rgba => Box::new([0u32; 256]),
        };
        let emu = EmuThread::spawn(
            Box::new(core),
            Arc::clone(&shared_fb),
            Arc::clone(&frame_ready),
            palette,
        );

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

    pub fn metrics(&self) -> ConsoleMetrics {
        let mut guard = self.metrics.lock().unwrap_or_else(|e| {
            log::warn!("metrics mutex poisoned, recovering");
            e.into_inner()
        });
        guard.frame_counter = self.emu.frame_count();
        guard.emulation_fps = self.emu.fps();
        *guard
    }

    pub fn video_frame_handle(&self) -> VideoFrameHandle {
        let logical_w = self.render_profile.logical_size.width;
        let bpp = self.render_profile.frame_format.bytes_per_pixel();
        let bytes = self.disp_fb.as_ref();
        VideoFrameHandle {
            width: logical_w as u32,
            height: self.render_profile.logical_size.height as u32,
            stride_bytes: logical_w * bpp,
            bytes: Arc::from(bytes),
        }
    }

    pub fn snapshot(&self) -> SystemRuntimeSnapshot {
        SystemRuntimeSnapshot {
            metrics: self.metrics(),
            video_frame: Some(self.video_frame_handle()),
        }
    }

    pub fn set_volume(&self, volume: f32) {
        if self.emu.send(EmuCommand::SetVolume(volume)).is_err() {
            log::warn!("set_volume: emu thread channel unavailable");
        }
    }

    pub fn resume(&self) {
        if self.emu.send(EmuCommand::Resume).is_err() {
            return;
        }
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = false,
            Err(e) => log::warn!("metrics lock poisoned in resume: {e}"),
        }
    }

    pub fn pause(&self) {
        if self.emu.send(EmuCommand::Pause).is_err() {
            return;
        }
        match self.metrics.lock() {
            Ok(mut guard) => guard.paused = true,
            Err(e) => log::warn!("metrics lock poisoned in pause: {e}"),
        }
    }

    // TODO(Phase 12): CoreConfig に CoreOptions を統合し、request.core_options を反映させる。
    // 現状は NesConsoleCore::load も config を無視する。
    pub fn load(
        &self,
        media: &MediaObject,
        _request: &ResolvedLoadRequest,
    ) -> Result<(), EmuCoreError> {
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
            .map_err(|e| EmuCoreError::Core(e.to_string()));
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

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::MapperSave { reply: reply_tx })
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Core(e.to_string()))
    }

    pub fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), EmuCoreError> {
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
            .map_err(|e| EmuCoreError::Core(e.to_string()))
    }

    pub fn export_state(&self) -> Result<RuntimeStateExport, EmuCoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SaveState { reply: reply_tx })
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        let core_state = reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| EmuCoreError::Core(e.to_string()))?;

        let state_blob = save_state_with_header(core_state);

        let Ok(guard) = self.emu.shared_frame_buffer().lock() else {
            return Err(EmuCoreError::Core(
                "emu thread shared frame buffer lock failed".into(),
            ));
        };
        let w = guard.width();
        let h = guard.height();
        let rgba = if w > 0 && h > 0 {
            if let Some(palette) = guard.palette() {
                let indices = guard.as_ref();
                let mut rgba = Vec::with_capacity(w * h * 4);
                for &idx in indices.iter().take(w * h) {
                    let color = palette[idx as usize];
                    rgba.push((color >> 24) as u8);
                    rgba.push((color >> 16) as u8);
                    rgba.push((color >> 8) as u8);
                    rgba.push(color as u8);
                }
                rgba
            } else {
                guard.as_ref().to_vec()
            }
        } else {
            Vec::new()
        };
        drop(guard);

        let preview = if w > 0 && h > 0 {
            Some(PreviewFrame {
                width: w as u32,
                height: h as u32,
                rgba,
            })
        } else {
            None
        };

        Ok(RuntimeStateExport {
            state_blob,
            preview,
        })
    }

    pub fn canonical_media_identity(&self) -> Option<CanonicalMediaIdentity> {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .emu
            .send(EmuCommand::Identity { reply: reply_tx })
            .is_err()
        {
            return None;
        }
        reply_rx.recv().ok().and_then(|r| r.ok())
    }

    pub fn import_state(&self, bytes: &[u8]) -> Result<(), EmuCoreError> {
        let core_bytes = match load_state_from_header(bytes) {
            Ok(inner) => inner.to_vec(),
            Err(_) => match rmp_serde::from_slice::<ConsoleStatePayload>(bytes) {
                Ok(payload) => payload.core_state,
                Err(_) => bytes.to_vec(),
            },
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::LoadState(Box::new(StateDataCommand {
                data: core_bytes,
                reply: reply_tx,
            })))
            .map_err(|_| EmuCoreError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| EmuCoreError::NoReply)?
            .map_err(|e| {
                EmuCoreError::Core(format!(
                    "state import failed (tried SaveStateHeader format, then \
                     ConsoleStatePayload format, then raw): {e}"
                ))
            })
    }
}
