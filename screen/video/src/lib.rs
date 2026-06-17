use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoFrameFormat {
    Rgba,
    Palette,
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
        }
    }

    /// 指定サイズにリサイズ（capacity 内ならゼロアロケーション）
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        let bpp = self.format.bytes_per_pixel();
        self.stride = match self.format {
            PixelFormat::Rgba => ((width * bpp).max(1) + 255) & !255,
            PixelFormat::PaletteIndex { .. } => width * bpp,
        };
        self.data.resize(self.stride * height, 0);
    }

    /// データバッファを指定バイト数にリサイズする。
    /// width/height/stride は更新せず、data のみを拡張する。
    /// ScreenBuffer の出力サイズ (frame_len()) に合わせるために使用する。
    pub fn resize_data(&mut self, len: usize) {
        self.data.resize(len, 0);
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

    /// PaletteIndex 形式の場合、先頭 64 エントリを RGBA8 バイト列 (256B) に変換して返す。
    /// GPU texture (Rgba8Uint) へのアップロード用。
    pub fn palette_as_rgba8(&self) -> Option<[u8; 256]> {
        match &self.format {
            PixelFormat::PaletteIndex { palette } => {
                let mut bytes = [0u8; 256];
                for (i, &entry) in palette.iter().enumerate().take(64) {
                    bytes[i * 4] = (entry >> 24) as u8;
                    bytes[i * 4 + 1] = (entry >> 16) as u8;
                    bytes[i * 4 + 2] = (entry >> 8) as u8;
                    bytes[i * 4 + 3] = entry as u8;
                }
                Some(bytes)
            }
            PixelFormat::Rgba => None,
        }
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

// === 既存: Screen trait（あとで削除予定） ===

pub trait Screen {
    fn push(&mut self, palette: u8);

    #[inline]
    fn push_many(&mut self, palette: u8, count: u16) {
        for _ in 0..count {
            self.push(palette);
        }
    }

    fn render(&mut self);
}
