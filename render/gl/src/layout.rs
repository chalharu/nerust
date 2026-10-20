//! Validated frame geometry. `FrameLayout::checked` returns a domain
//! [`FrameError`]; the view wraps it via `#[from]`.

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
    /// Surface or content size unusable for viewport fitting.
    #[error("invalid viewport {w}x{h}")]
    InvalidViewport { w: i32, h: i32 },
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

/// Fit the game into the surface minus reserved OSD strips. Pure.
/// GL viewports start at the bottom-left; ratios run in `f64`, narrowing once.
pub(crate) fn fit_viewport(
    surface_w: i32,
    surface_h: i32,
    strip: nerust_render_traits::ReservedStrip,
    logical_w: i32,
    logical_h: i32,
) -> Result<([i32; 4], [f32; 16]), FrameError> {
    if surface_w <= 0 || surface_h <= 0 {
        return Err(FrameError::InvalidViewport {
            w: surface_w,
            h: surface_h,
        });
    }
    if logical_w <= 0 || logical_h <= 0 {
        return Err(FrameError::InvalidViewport {
            w: logical_w,
            h: logical_h,
        });
    }
    let height_i = surface_h.max(1);
    // Exact: `surface_*` are validated positive `i32`, always `< u32::MAX`.
    debug_assert!(surface_w > 0 && surface_h > 0);
    let top = strip.top.min(height_i as u32) as i32;
    let bottom = strip.bottom.min(height_i as u32 - top as u32) as i32;
    let area_height = (height_i - top - bottom).max(1);
    let area_width = surface_w.max(1);
    // `f64` ratios: `as f32` division on 8K-class widths already
    // loses sub-pixel fit (e.g. 256/7680 rounds early).
    let window_aspect = area_width as f64 / area_height as f64;
    let content_aspect = logical_w as f64 / logical_h as f64;
    if !window_aspect.is_finite() || !content_aspect.is_finite() || content_aspect == 0.0 {
        return Err(FrameError::InvalidViewport {
            w: surface_w,
            h: surface_h,
        });
    }
    let (scale_x, scale_y) = if window_aspect > content_aspect {
        (content_aspect / window_aspect, 1.0)
    } else {
        (1.0, window_aspect / content_aspect)
    };
    if !scale_x.is_finite() || !scale_y.is_finite() {
        return Err(FrameError::InvalidViewport {
            w: surface_w,
            h: surface_h,
        });
    }
    let (sx, sy) = (scale_x as f32, scale_y as f32);
    Ok((
        [0, bottom, area_width, area_height],
        [
            sx, 0.0, 0.0, 0.0, //
            0.0, sy, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::{FrameError, fit_viewport};

    #[test]
    fn fit_exact_aspect_is_identity() {
        let (viewport, mat) = fit_viewport(
            256,
            240,
            nerust_render_traits::ReservedStrip::default(),
            256,
            240,
        )
        .unwrap();
        assert_eq!(viewport, [0, 0, 256, 240]);
        assert_eq!(mat[0], 1.0);
        assert_eq!(mat[5], 1.0);
    }

    #[test]
    fn fit_letterboxes_wide_window() {
        // 512x240 surface, 256x240 content: pillarbox, x scale 0.5.
        let (viewport, mat) = fit_viewport(
            512,
            240,
            nerust_render_traits::ReservedStrip::default(),
            256,
            240,
        )
        .unwrap();
        assert_eq!(viewport, [0, 0, 512, 240]);
        assert!((mat[0] - 0.5).abs() < 1e-6);
        assert_eq!(mat[5], 1.0);
    }

    #[test]
    fn fit_respects_bottom_strip() {
        let strip = nerust_render_traits::ReservedStrip { top: 0, bottom: 8 };
        let (viewport, _) = fit_viewport(64, 64, strip, 64, 64).unwrap();
        assert_eq!(viewport, [0, 8, 64, 56]);
    }

    #[test]
    fn fit_rejects_degenerate() {
        assert!(matches!(
            fit_viewport(0, 64, Default::default(), 64, 64),
            Err(FrameError::InvalidViewport { .. })
        ));
        assert!(matches!(
            fit_viewport(64, 64, Default::default(), 0, 64),
            Err(FrameError::InvalidViewport { .. })
        ));
    }

    #[test]
    fn frame_errors_name_sizes() {
        assert_eq!(
            FrameError::TooShort { need: 8, have: 2 }.to_string(),
            "frame buffer too small: need 8, have 2"
        );
        assert_eq!(
            FrameError::StrideTooWide { have: 9 }.to_string(),
            "frame stride overflows i32 (have 9 bytes)"
        );
        assert_eq!(
            FrameError::DimensionsOverflow { have: 3 }.to_string(),
            "frame dimensions overflow (have 3 bytes)"
        );
        assert_eq!(
            FrameError::StrideOverflow { have: 4 }.to_string(),
            "frame stride overflow (have 4 bytes)"
        );
        assert_eq!(
            FrameError::InvalidViewport { w: 0, h: 64 }.to_string(),
            "invalid viewport 0x64"
        );
    }

    #[test]
    fn checked_rejects_bad_strides_and_short_buffers() {
        use super::FrameLayout;
        use nerust_render_traits::logical::LogicalSize;
        let fs = LogicalSize {
            width: 64,
            height: 64,
        };
        // Stride not divisible by bpp refuses.
        assert!(matches!(
            FrameLayout::checked(fs, 4, 257, 64 * 64 * 4, false),
            Err(FrameError::StrideOverflow { .. })
        ));
        // Short buffer refuses on the padded path.
        assert!(matches!(
            FrameLayout::checked(fs, 4, 260, 100, true),
            Err(FrameError::TooShort { .. })
        ));
        // Stride wider than i32 refuses instead of truncating.
        assert!(matches!(
            FrameLayout::checked(fs, 1, u32::MAX as usize + 1, usize::MAX, true),
            Err(FrameError::StrideTooWide { .. })
        ));
    }

    #[test]
    fn fit_scales_tall_content_and_top_strip() {
        // 64x64 surface, 64x128 content: pillar fit, x scale 0.5.
        let (viewport, mat) = fit_viewport(
            64,
            64,
            nerust_render_traits::ReservedStrip::default(),
            64,
            128,
        )
        .unwrap();
        assert_eq!(viewport, [0, 0, 64, 64]);
        assert!((mat[0] - 0.5).abs() < 1e-6);
        assert_eq!(mat[5], 1.0);
        // Top strip pushes the drawable rect down.
        let strip = nerust_render_traits::ReservedStrip { top: 8, bottom: 0 };
        let (viewport, _) = fit_viewport(64, 64, strip, 64, 64).unwrap();
        assert_eq!(viewport, [0, 0, 64, 56]);
    }
}
