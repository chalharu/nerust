//! Validated frame geometry: the orchestration layer's shape check lives
//! here, not in the view adapter. `FrameLayout::checked` returns a
//! domain [`FrameError`]; the view wraps it via `#[from]` so renderer
//! orchestration never names view internals.

/// Frame validation faults. Structured so logs name the fault instead
/// of a `&'static str` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// Width/height/row-bytes math overflowed.
    #[error("frame dimensions overflow (have {have} bytes)")]
    DimensionsOverflow { have: usize },
    /// Stride math overflowed.
    #[error("frame stride overflow (have {have} bytes)")]
    StrideOverflow { have: usize },
    /// Pixel stride does not fit the GL `i32` upload path.
    #[error("frame stride overflows i32 (have {have} bytes)")]
    StrideTooWide { have: usize },
    /// Buffer shorter than the validated need.
    #[error("frame buffer too small: need {need}, have {have}")]
    TooShort { need: usize, have: usize },
}

/// Checked frame geometry shared by the attach guard and the three
/// upload paths: one validated computation, no `usize::MAX` sentinel.
pub(crate) struct FrameLayout {
    pub(crate) width: usize,
    pub(crate) height: usize,
    /// Tight row bytes (`width * bpp`).
    pub(crate) row_bytes: usize,
    /// Source stride in bytes.
    pub(crate) stride: usize,
    /// Source bytes the chosen path consumes.
    pub(crate) need: usize,
    /// Stride in pixels for the `UNPACK_ROW_LENGTH` path.
    pub(crate) stride_px: i32,
}

impl FrameLayout {
    pub(crate) fn checked(
        fs: nerust_render_traits::logical::LogicalSize,
        bpp: usize,
        stride: usize,
        data_len: usize,
        unpack_row_length: bool,
    ) -> Result<Self, FrameError> {
        if bpp == 0 {
            return Err(FrameError::DimensionsOverflow { have: data_len });
        }
        let row_bytes = fs
            .width
            .checked_mul(bpp)
            .ok_or(FrameError::DimensionsOverflow { have: data_len })?;
        if !stride.is_multiple_of(bpp) {
            return Err(FrameError::StrideOverflow { have: data_len });
        }
        let stride_px = stride
            .checked_div(bpp)
            .ok_or(FrameError::StrideOverflow { have: data_len })?;
        let need = if unpack_row_length {
            fs.height.checked_sub(1).map_or(Ok(0), |rows| {
                rows.checked_mul(stride)
                    .and_then(|tail| tail.checked_add(row_bytes))
                    .ok_or(FrameError::StrideOverflow { have: data_len })
            })?
        } else if stride == row_bytes {
            fs.width
                .checked_mul(fs.height)
                .and_then(|px| px.checked_mul(bpp))
                .ok_or(FrameError::DimensionsOverflow { have: data_len })?
        } else {
            stride
                .checked_mul(fs.height)
                .ok_or(FrameError::StrideOverflow { have: data_len })?
        };
        if data_len < need {
            return Err(FrameError::TooShort {
                need,
                have: data_len,
            });
        }
        Ok(Self {
            width: fs.width,
            height: fs.height,
            row_bytes,
            stride,
            need,
            stride_px: i32::try_from(stride_px)
                .map_err(|_| FrameError::StrideTooWide { have: data_len })?,
        })
    }
}
