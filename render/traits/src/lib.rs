pub mod filter;
/// Backend-independent validation of a visible input rectangle. Extra source
/// rows/columns and trailing bytes are allowed; final-row padding is not required.
pub mod frame {
    use crate::{FrameBuffer, VideoFrameFormat, logical::LogicalSize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FrameError {
        ProfileMismatch,
        InvalidDimensions,
        InvalidStride,
        TooShort { need: usize, have: usize },
    }

    impl std::fmt::Display for FrameError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::ProfileMismatch => {
                    f.write_str("frame format or dimensions do not match the render profile")
                }
                Self::InvalidDimensions => f.write_str("invalid or overflowing frame dimensions"),
                Self::InvalidStride => f.write_str("invalid or overflowing frame stride"),
                Self::TooShort { need, have } => {
                    write!(f, "frame buffer too small: need {need}, have {have}")
                }
            }
        }
    }

    impl std::error::Error for FrameError {}

    /// Checked geometry. Private fields prevent bypassing validation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FrameLayout {
        size: LogicalSize,
        row_bytes: usize,
        stride: usize,
        need: usize,
    }

    impl FrameLayout {
        pub fn validate(
            frame: &FrameBuffer,
            size: LogicalSize,
            format: VideoFrameFormat,
        ) -> Result<Self, FrameError> {
            let compatible = matches!(
                (format, frame.format()),
                (VideoFrameFormat::Rgba, crate::PixelFormat::Rgba)
                    | (
                        VideoFrameFormat::Palette,
                        crate::PixelFormat::PaletteIndex { .. }
                    )
            );
            if !compatible || frame.width() < size.width || frame.height() < size.height {
                return Err(FrameError::ProfileMismatch);
            }
            Self::checked(
                size,
                format.bytes_per_pixel(),
                frame.stride(),
                frame.as_ref().len(),
            )
        }

        /// Raw-input validation for upload APIs without a FrameBuffer.
        pub fn checked(
            size: LogicalSize,
            bpp: usize,
            stride: usize,
            data_len: usize,
        ) -> Result<Self, FrameError> {
            if size.width == 0 || size.height == 0 || bpp == 0 {
                return Err(FrameError::InvalidDimensions);
            }
            let row_bytes = size
                .width
                .checked_mul(bpp)
                .ok_or(FrameError::InvalidDimensions)?;
            if stride < row_bytes || !stride.is_multiple_of(bpp) {
                return Err(FrameError::InvalidStride);
            }
            let need = (size.height - 1)
                .checked_mul(stride)
                .and_then(|tail| tail.checked_add(row_bytes))
                .ok_or(FrameError::InvalidStride)?;
            if data_len < need {
                return Err(FrameError::TooShort {
                    need,
                    have: data_len,
                });
            }
            Ok(Self {
                size,
                row_bytes,
                stride,
                need,
            })
        }

        pub fn size(self) -> LogicalSize {
            self.size
        }
        pub fn row_bytes(self) -> usize {
            self.row_bytes
        }
        pub fn stride(self) -> usize {
            self.stride
        }
        pub fn required_bytes(self) -> usize {
            self.need
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::PixelFormat;

        #[test]
        fn checked_rejects_malformed_layouts_without_overflow() {
            let size = LogicalSize {
                width: 2,
                height: 2,
            };
            for stride in [0, 4, 9, usize::MAX] {
                assert!(FrameLayout::checked(size, 4, stride, usize::MAX).is_err());
            }
            assert!(
                FrameLayout::checked(
                    LogicalSize {
                        width: usize::MAX,
                        height: 2
                    },
                    4,
                    16,
                    32
                )
                .is_err()
            );
            assert!(
                FrameLayout::checked(
                    LogicalSize {
                        width: 0,
                        height: 2
                    },
                    4,
                    16,
                    32
                )
                .is_err()
            );
            assert_eq!(
                FrameLayout::checked(size, 4, 12, 19),
                Err(FrameError::TooShort { need: 20, have: 19 })
            );
            assert_eq!(
                FrameLayout::checked(size, 4, 12, 20)
                    .unwrap()
                    .required_bytes(),
                20
            );
            assert!(FrameLayout::checked(size, 4, 12, 99).is_ok());
        }

        #[test]
        fn metadata_and_payload_share_one_contract() {
            let size = LogicalSize {
                width: 2,
                height: 2,
            };
            let mut frame = FrameBuffer::with_capacity(4, 4, PixelFormat::Rgba);
            frame.resize(4, 4);
            assert!(FrameLayout::validate(&frame, size, VideoFrameFormat::Rgba).is_ok());
            assert!(FrameLayout::validate(&frame, size, VideoFrameFormat::Palette).is_err());
            frame.resize_data(264);
            assert!(FrameLayout::validate(&frame, size, VideoFrameFormat::Rgba).is_ok());
            frame.resize_data(263);
            assert!(FrameLayout::validate(&frame, size, VideoFrameFormat::Rgba).is_err());
        }
    }
}
pub mod logical;
pub mod physical;
#[cfg(feature = "renderer")]
pub mod renderer;
pub mod rgb;

use crate::{logical::LogicalSize, physical::PhysicalSize, rgb::RGB};
pub use frame::{FrameError, FrameLayout};

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

/// Physical pixels reserved at the surface edges for OSD strips.
/// The game viewport fits into the remaining rect; strips stay clear
/// color for the frontend's overlay labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReservedStrip {
    pub top: u32,
    pub bottom: u32,
}

impl ReservedStrip {
    pub fn height_loss(&self, surface_height: u32) -> u32 {
        self.top.saturating_add(self.bottom).min(surface_height)
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

impl VideoRenderProfile {
    /// RGBA frames are CPU-filtered; palette frames are the original indices.
    pub fn input_size(&self) -> LogicalSize {
        self.frame_format
            .input_size(self.source_logical_size, self.logical_size)
    }
}

impl VideoFrameFormat {
    pub fn input_size(self, source: LogicalSize, filtered: LogicalSize) -> LogicalSize {
        match self {
            Self::Rgba => filtered,
            Self::Palette => source,
        }
    }
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

    pub fn input_size(&self) -> LogicalSize {
        self.frame_format()
            .input_size(self.source_logical_size(), self.logical_size())
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
        /// RGBA palette entries (u32 = 0xRRGGBBAA)
        palette: Box<[u32]>,
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

/// フレームバッファ（事前確保・再利用）。Console が `resize` で準備し
/// `as_mut()` で書込・GUI が `as_ref()` で読取。`Arc<Mutex>` + `mem::swap`
/// で受け渡す。stride は1行バイト数（RGBA は 256B 境界）。
#[derive(Debug, Clone)]
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

    ///  PaletteIndex 形式の場合、パレットテーブルへの参照を返す。
    pub fn palette(&self) -> Option<&[u32]> {
        match &self.format {
            PixelFormat::PaletteIndex { palette } => Some(palette.as_ref()),
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
