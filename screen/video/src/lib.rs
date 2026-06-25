pub mod filter;
pub mod logical;
pub mod physical;
pub mod renderer;
pub mod rgb;

pub use crate::{
    filter::{
        BLACK_PALETTE_INDEX, FilterFunc, FilterType, NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH,
        NesFilter, PALETTE_TEXTURE_WIDTH,
        presentation::{
            ConsoleVideoAssets, EncodedNtscTextures, EncodedPackedNtscTexture, FilterLayout,
            NesVideoAssets, VideoFilterPipeline, VideoPresentationPipelineKind,
        },
    },
    logical::LogicalSize,
    physical::PhysicalSize,
    renderer::{
        OpaqueError, RenderResult, Renderer, RendererConfig, RendererError, RendererFactory,
        Surface,
    },
    rgb::RGB,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

impl SurfaceSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoFrameFormat {
    Rgba,
    Palette,
}

impl VideoFrameFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            VideoFrameFormat::Rgba => 4,
            VideoFrameFormat::Palette => 1,
        }
    }
}

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrameHandle {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub bytes: Arc<[u8]>,
}

impl VideoFrameHandle {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct VideoRenderProfile {
    pub source_logical_size: LogicalSize,
    pub logical_size: LogicalSize,
    pub physical_size: PhysicalSize,
    pub frame_format: VideoFrameFormat,
    pub ntsc_packed_rgba8: Option<Box<[u8]>>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoFrameSpec {
    frame_format: VideoFrameFormat,
    source_logical_size: LogicalSize,
    logical_size: LogicalSize,
    physical_size: PhysicalSize,
}

impl VideoFrameSpec {
    pub fn new(
        frame_format: VideoFrameFormat,
        source_logical_size: LogicalSize,
        logical_size: LogicalSize,
        physical_size: PhysicalSize,
    ) -> Self {
        Self {
            frame_format,
            source_logical_size,
            logical_size,
            physical_size,
        }
    }

    pub fn frame_format(&self) -> VideoFrameFormat {
        self.frame_format
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.source_logical_size
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.physical_size
    }
}

#[derive(Debug, Clone)]
pub struct VideoPresentation {
    frame_spec: VideoFrameSpec,
}

impl VideoPresentation {
    pub fn new(frame_spec: VideoFrameSpec) -> Self {
        Self { frame_spec }
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.frame_spec.source_logical_size()
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.frame_spec.logical_size()
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.frame_spec.physical_size()
    }

    pub fn frame_format(&self) -> VideoFrameFormat {
        self.frame_spec.frame_format()
    }

    pub fn is_palette_frame(&self) -> bool {
        matches!(self.frame_spec.frame_format(), VideoFrameFormat::Palette)
    }
}

// === 新設計: FrameBuffer / PixelFormat ===

/// ピクセルフォーマット
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelFormat {
    /// 4 bytes/pixel, RGBA各8bit. GPUにそのまま転送.
    Rgba,

    /// 1 byte/pixel, palette index + palette LUT.
    PaletteIndex {
        /// 256エントリのRGBAパレット (u32 = 0xRRGGBBAA)
        palette: Box<[u32; 256]>,
    },
}

impl PixelFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            PixelFormat::Rgba => 4,
            PixelFormat::PaletteIndex { .. } => 1,
        }
    }
}

/// フレームバッファ（事前確保・再利用）
///
/// Console が `resize(w, h)` で準備し、`as_mut()` でピクセルを書き込む。
/// GUI は `as_ref()` で読み取る。スレッド間の受け渡しは
/// `Arc<Mutex<FrameBuffer>>` + `mem::swap` で行う（ゼロコピー）。
#[derive(Debug)]
pub struct FrameBuffer {
    data: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    cursor: usize,
}

impl FrameBuffer {
    /// 最大解像度のバッファを事前確保する
    pub fn with_capacity(max_width: usize, max_height: usize, format: PixelFormat) -> Self {
        let bpp = format.bytes_per_pixel();
        Self {
            data: Vec::with_capacity(max_width * max_height * bpp),
            width: 0,
            height: 0,
            stride: 0,
            format,
            cursor: 0,
        }
    }

    /// 指定サイズにリサイズ（capacity 内ならゼロアロケーション）
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.cursor = 0;
        let bpp = self.format.bytes_per_pixel();
        self.stride = match self.format {
            PixelFormat::Rgba => ((width * bpp).max(1) + 255) & !255,
            PixelFormat::PaletteIndex { .. } => width * bpp,
        };
        self.data.resize(self.stride * height, 0);
    }

    /// PPU が palette index を 1 ピクセル書き込む。
    /// バッファ不足時は警告ログを出力して無視する。
    pub fn push(&mut self, value: u8) {
        if self.cursor >= self.data.len() {
            log::warn!(
                "FrameBuffer::push: cursor {} out of bounds (len {})",
                self.cursor,
                self.data.len()
            );
            return;
        }
        self.data[self.cursor] = value;
        self.cursor += 1;
    }

    /// PPU が同一 palette index を連続書き込みする。
    /// バッファ不足時は警告ログを出力して無視する。
    pub fn push_many(&mut self, value: u8, count: u16) {
        let end = self.cursor + count as usize;
        if end > self.data.len() {
            log::warn!(
                "FrameBuffer::push_many: cursor {} + count {} out of bounds (len {})",
                self.cursor,
                count,
                self.data.len()
            );
            return;
        }
        self.data[self.cursor..end].fill(value);
        self.cursor = end;
    }

    /// フレーム完了を通知する（cursor を先頭に戻す）。
    pub fn render(&mut self) {
        self.cursor = 0;
    }

    /// バッファ全体をゼロで埋める。
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.cursor = 0;
    }

    /// データバッファを指定バイト数にリサイズする。
    /// width/height/stride は更新せず、data のみを拡張する。
    /// ScreenBuffer の出力サイズ (frame_len()) に合わせるために使用する。
    pub fn resize_data(&mut self, len: usize) {
        self.data.resize(len, 0);
    }

    pub fn set_format(&mut self, format: PixelFormat) {
        self.format = format;
        if self.width > 0 {
            self.resize(self.width, self.height);
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn format(&self) -> &PixelFormat {
        &self.format
    }

    /// PaletteIndex 形式の場合、パレットテーブルへの参照を返す。
    pub fn palette(&self) -> Option<&[u32; 256]> {
        match &self.format {
            PixelFormat::PaletteIndex { palette } => Some(palette.as_ref()),
            PixelFormat::Rgba => None,
        }
    }

    pub fn palette_mut(&mut self) -> Option<&mut [u32; 256]> {
        match &mut self.format {
            PixelFormat::PaletteIndex { palette } => Some(palette.as_mut()),
            PixelFormat::Rgba => None,
        }
    }

    /// 先頭64エントリを RGBA8 バイト列 (256B) に変換。GPU upload 用。
    /// PaletteIndex 形式以外の場合は None。
    pub fn palette_as_rgba8(&self) -> Option<[u8; 256]> {
        let palette = self.palette()?;
        let mut rgba8 = [0u8; 256];
        for (i, &color) in palette.iter().enumerate().take(64) {
            let pos = i * 4;
            rgba8[pos] = (color >> 24) as u8; // R
            rgba8[pos + 1] = (color >> 16) as u8; // G
            rgba8[pos + 2] = (color >> 8) as u8; // B
            rgba8[pos + 3] = color as u8; // A
        }
        Some(rgba8)
    }
}

impl AsRef<[u8]> for FrameBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl AsMut<[u8]> for FrameBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
