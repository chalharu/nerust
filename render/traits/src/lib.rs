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

// Viewport vocabulary now lives in wakui (framework-owned); the engine
// re-exports it so existing `nerust_render_traits::` paths keep resolving.
// New code names `wakui::viewport` paths directly.
pub use wakui::viewport::{
    FrameBuffer, PixelFormat, RenderedImage, ReservedStrip, SurfaceSize, VideoFrameFormat,
    VideoFrameHandle, VideoFrameSpec, VideoPresentation, VideoRenderProfile,
};
pub mod logical {
    pub use wakui::viewport::geometry::LogicalSize;
}
pub mod physical {
    pub use wakui::viewport::geometry::PhysicalSize;
}
#[cfg(feature = "renderer")]
pub mod renderer {
    pub use wakui::viewport::renderer::*;
}
pub mod rgb;

use crate::{logical::LogicalSize, physical::PhysicalSize, rgb::RGB};
pub use frame::{FrameError, FrameLayout};
