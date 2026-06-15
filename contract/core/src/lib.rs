pub mod audio;
pub mod device;
pub mod mirror;
pub mod options;
pub mod persistence;
pub mod rom;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub use nerust_screen_video::PixelFormat;

// ---------------------------------------------------------------------------
// CoreError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("ROM parse failed: {0}")]
    RomParse(String),
    #[error("core error: {0}")]
    Core(String),
    #[error("no ROM loaded")]
    NoRomLoaded,
}

// ---------------------------------------------------------------------------
// VideoSignalKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSignalKind {
    Ntsc,
    Rgb,
    Lcd,
    Other,
}

// ---------------------------------------------------------------------------
// CoreCapabilities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CoreCapabilities {
    pub output_formats: Vec<PixelFormat>,
    pub video_signal: VideoSignalKind,
}

// ---------------------------------------------------------------------------
// GpuCommand
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum GpuCommand {
    /// RGBA スロットをそのまま表示
    Blit { slot: u32 },
    /// PaletteIndex スロットを GPU の固定パレットでデコード
    PaletteDecode { slot: u32 },
    /// パレットデータを GPU にアップロード
    UploadPalette { slot: u32, data: Box<[u32; 256]> },
    /// テクスチャ更新
    UploadTexture {
        slot: u32,
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
    /// 3D描画
    DrawMesh {
        vertices: Vec<Vertex>,
        indices: Vec<u16>,
        textures: Vec<u32>,
    },
}

#[derive(Clone)]
pub struct GpuCommandList {
    pub commands: Vec<GpuCommand>,
}

// ---------------------------------------------------------------------------
// GPU resource identifiers (forward declarations)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaletteId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
    // future: Bgra8, R8, Rg8, …
}

#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

// ---------------------------------------------------------------------------
// Region
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Ntsc,
    Pal,
}

// ---------------------------------------------------------------------------
// ControllerKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerKind {
    None,
    Standard,
    Zapper,
}

// ---------------------------------------------------------------------------
// CoreConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub region: Option<Region>,
    pub bios_paths: HashMap<String, PathBuf>,
    pub controllers: HashMap<usize, ControllerKind>,
}

// ---------------------------------------------------------------------------
// EmuCommand
// ---------------------------------------------------------------------------

pub enum EmuCommand {
    RenderFrame,
    Pause,
    Resume,
    Reset,
    SaveState(Sender<Result<Vec<u8>, CoreError>>),
    LoadState(Vec<u8>, Sender<Result<(), CoreError>>),
    ApplyInputState(Vec<u8>),
    Quit,
}

// ---------------------------------------------------------------------------
// ConsoleCore trait
// ---------------------------------------------------------------------------

pub trait ConsoleCore: Send {
    // -- video --
    fn capabilities(&self) -> CoreCapabilities;
    /// 目標フレーム間隔（NTSC = ≈16.67ms, PAL = 20ms）
    fn frame_interval(&self) -> Duration {
        Duration::from_nanos(16_666_667)
    }
    /// フレームをレンダリングする。
    /// frame_slot はレンダラから渡された書き込み可能バッファ。
    fn render_frame(&mut self, frame_slot: &mut [u8]) -> Result<GpuCommandList, CoreError>;

    // -- input --
    fn apply_input_state(&mut self, _bytes: &[u8]) {}
    // -- peripherals --
    fn attach_device(&mut self, port: usize, device: Box<dyn device::Device>);
    fn detach_device(&mut self, port: usize);

    // -- lifecycle --
    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError>;
    fn unload(&mut self);
    fn reset(&mut self);

    // -- pause --
    fn paused(&self) -> bool;
    fn set_paused(&mut self, paused: bool);

    // -- save states --
    fn save_state(&self) -> Result<Vec<u8>, CoreError>;
    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError>;

    // -- rewind (default: not supported) --
    fn rewind_state_size(&self) -> Option<usize> {
        None
    }
    fn rewind_save(&self, _buf: &mut [u8]) {
        panic!("rewind not supported")
    }
    fn rewind_restore(&mut self, _buf: &[u8]) {
        panic!("rewind not supported")
    }
}
