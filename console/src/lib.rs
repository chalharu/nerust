pub mod controller;
pub mod state;
pub mod video;

use self::state::RuntimeStateExport;
use self::video::ConsoleVideo;
use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::channel::frame_channel;
use nerust_contract_core::options::CoreOptions;
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_contract_core::{CoreConfig, EmuCommand};
use nerust_contract_emuthread::EmuThread;
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_nes_core::controller::Controller;
use nerust_screen_video::FilterType;
use nerust_screen_video::LogicalSize;
use nerust_screen_video::PhysicalSize;
use nerust_screen_video::{FrameBuffer, PixelFormat};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleMetrics {
    pub frame_counter: u64,
    pub emulation_fps: f32,
    pub speed_multiplier: f32,
    pub loaded: bool,
    pub paused: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConsoleError {
    #[error("console worker thread is unavailable")]
    WorkerUnavailable,
    #[error("ROM parse failed: {0}")]
    RomParse(String),
    #[error("no ROM loaded")]
    NoRomLoaded,
    #[error("{0}")]
    Core(String),
}

#[allow(missing_debug_implementations)]
pub struct Console {
    emu: EmuThread<NesConsoleCore>,
    video: ConsoleVideo,
    metrics: Arc<Mutex<ConsoleMetrics>>,
}

impl Console {
    /// NES 用の新規 Console を作成する (GPU palette decode パス)。
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
        let render_profile = video::VideoRenderProfile {
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
        render_profile: video::VideoRenderProfile,
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
        {
            let mut guard = shared_fb.lock().unwrap();
            guard.resize(src_w, src_h);
            guard.resize_data(src_w * src_h);
        }

        let mut disp_fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format.clone());
        disp_fb.resize(src_w, src_h);
        disp_fb.resize_data(src_w * src_h);

        let (_console_ch, renderer_ch) = frame_channel(4);

        let metrics = Arc::new(Mutex::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        }));

        let core = NesConsoleCore::new_empty(controller, speaker);
        let emu = EmuThread::spawn(core, Arc::clone(&shared_fb));

        let _ = emu.send(EmuCommand::RenderFrame);

        Self {
            emu,
            video: ConsoleVideo::new(render_profile, shared_fb, disp_fb, renderer_ch),
            metrics,
        }
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.video().render_profile().logical_size
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.video().render_profile().source_logical_size
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.video().render_profile().physical_size
    }

    pub fn video(&self) -> &ConsoleVideo {
        &self.video
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.video.with_frame_buffer(f)
    }

    /// 共有バッファから表示バッファに最新フレームを引き取る。
    /// 新しいフレームがあった場合は `true`。
    pub fn swap_frame_buffer(&mut self) -> bool {
        self.video.swap_frame_buffer()
    }

    /// 表示バッファへの参照を返す。
    pub fn frame_buffer(&self) -> &FrameBuffer {
        self.video.frame_buffer()
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        *self.metrics.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_volume(&self, volume: f32) {
        let _ = self.emu.send(EmuCommand::SetVolume(volume));
    }

    pub fn resume(&self) {
        let _ = self.emu.send(EmuCommand::Resume);
    }

    pub fn pause(&self) {
        let _ = self.emu.send(EmuCommand::Pause);
    }

    pub fn load(&self, data: Vec<u8>) -> Result<(), ConsoleError> {
        self.load_with_options(data, CoreOptions::default())
    }

    pub fn load_with_options(
        &self,
        data: Vec<u8>,
        _options: CoreOptions,
    ) -> Result<(), ConsoleError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        use std::collections::HashMap;
        self.emu
            .send(EmuCommand::Load {
                rom: data,
                config: CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                },
                reply: reply_tx,
            })
            .map_err(|_| ConsoleError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| ConsoleError::WorkerUnavailable)?
            .map_err(|e| ConsoleError::Core(e.to_string()))
    }

    pub fn unload(&self) -> Result<(), ConsoleError> {
        self.emu
            .send(EmuCommand::Unload)
            .map_err(|_| ConsoleError::WorkerUnavailable)
    }

    pub fn reset(&self) -> Result<(), ConsoleError> {
        self.emu
            .send(EmuCommand::Reset)
            .map_err(|_| ConsoleError::WorkerUnavailable)
    }

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, ConsoleError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::MapperSave { reply: reply_tx })
            .map_err(|_| ConsoleError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| ConsoleError::WorkerUnavailable)?
            .map_err(|e| ConsoleError::Core(e.to_string()))
    }

    pub fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::ImportMapperSave {
                data: bytes,
                reply: reply_tx,
            })
            .map_err(|_| ConsoleError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| ConsoleError::WorkerUnavailable)?
            .map_err(|e| ConsoleError::Core(e.to_string()))
    }

    pub fn export_state(&self) -> Result<RuntimeStateExport, ConsoleError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SaveState { reply: reply_tx })
            .map_err(|_| ConsoleError::WorkerUnavailable)?;
        let core_state = reply_rx
            .recv()
            .map_err(|_| ConsoleError::WorkerUnavailable)?
            .map_err(|e| ConsoleError::Core(e.to_string()))?;

        let guard = self.emu.shared_frame_buffer().lock().unwrap();
        let frame_data = guard.as_ref().to_vec();
        let w = guard.width();
        let h = guard.height();
        drop(guard);

        let preview = if w > 0 && h > 0 {
            Some(state::PreviewFrame {
                width: w as u32,
                height: h as u32,
                rgba: frame_data,
            })
        } else {
            None
        };

        Ok(RuntimeStateExport {
            state_blob: core_state,
            preview,
        })
    }

    pub fn canonical_media_identity(&self) -> Result<CanonicalMediaIdentity, ConsoleError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::Identity { reply: reply_tx })
            .map_err(|_| ConsoleError::WorkerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| ConsoleError::WorkerUnavailable)?
            .map_err(|e| ConsoleError::Core(e.to_string()))
    }

    pub fn import_state(&self, _bytes: Vec<u8>) -> Result<(), ConsoleError> {
        // Phase 7: implement state import via EmuCommand::LoadState
        Err(ConsoleError::Core(
            "state import not implemented in EmuThread path".into(),
        ))
    }
}
