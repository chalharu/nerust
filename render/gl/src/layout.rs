//! Validated frame geometry. `FrameLayout::checked` returns a domain
//! [`FrameError`]; the view wraps it via `#[from]`.

/// Frame validation faults. Structured so logs name the fault instead
/// of a `&'static str` tag. Port-level faults pass through unchanged;
/// only GL-specific context (byte counts, `i32` bounds) gets a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// Port validation fault; texts match the shared error exactly.
    #[error(transparent)]
    Shared(#[from] nerust_render_traits::FrameError),
    /// Width/height/row-bytes math overflowed.
    #[error("frame dimensions overflow (have {have} bytes)")]
    DimensionsOverflow { have: usize },
    /// Stride math overflowed.
    #[error("frame stride overflow (have {have} bytes)")]
    StrideOverflow { have: usize },
    /// Pixel stride does not fit the GL `i32` upload path.
    #[error("frame stride overflows i32 (have {have} bytes)")]
    StrideTooWide { have: usize },
    /// Surface or content size unusable for viewport fitting.
    #[error("invalid viewport {w}x{h}")]
    InvalidViewport { w: i32, h: i32 },
}

impl FrameError {
    fn from_shared(error: nerust_render_traits::FrameError, have: usize) -> Self {
        match error {
            nerust_render_traits::FrameError::ProfileMismatch
            | nerust_render_traits::FrameError::TooShort { .. } => Self::Shared(error),
            nerust_render_traits::FrameError::InvalidDimensions => {
                Self::DimensionsOverflow { have }
            }
            nerust_render_traits::FrameError::InvalidStride => Self::StrideOverflow { have },
        }
    }
}

/// Reject incompatible metadata before performing any GL work. Larger source
/// buffers are allowed; uploads consume only the profile's visible rectangle.
pub(crate) fn validate_frame(
    frame: &nerust_render_traits::FrameBuffer,
    size: nerust_render_traits::logical::LogicalSize,
    format: nerust_render_traits::VideoFrameFormat,
    unpack: bool,
) -> Result<FrameLayout, FrameError> {
    let data_len = frame.as_ref().len();
    let layout = nerust_render_traits::FrameLayout::validate(frame, size, format)
        .map_err(|error| FrameError::from_shared(error, data_len))?;
    FrameLayout::from_shared(layout, format.bytes_per_pixel(), data_len, unpack)
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
        let layout = nerust_render_traits::FrameLayout::checked(fs, bpp, stride, data_len)
            .map_err(|error| FrameError::from_shared(error, data_len))?;
        Self::from_shared(layout, bpp, data_len, unpack_row_length)
    }

    /// Shared validation owns byte geometry; only GL limits remain here.
    fn from_shared(
        layout: nerust_render_traits::FrameLayout,
        bpp: usize,
        data_len: usize,
        unpack_row_length: bool,
    ) -> Result<Self, FrameError> {
        let fs = layout.size();
        i32::try_from(fs.width)
            .and_then(|_| i32::try_from(fs.height))
            .map_err(|_| FrameError::StrideTooWide { have: data_len })?;
        let stride_px = if unpack_row_length {
            layout.stride() / bpp
        } else {
            fs.width
        };
        Ok(Self {
            width: fs.width,
            height: fs.height,
            row_bytes: layout.row_bytes(),
            stride: layout.stride(),
            need: layout.required_bytes(),
            stride_px: i32::try_from(stride_px)
                .map_err(|_| FrameError::StrideTooWide { have: data_len })?,
        })
    }
}

/// Fit the game into the surface minus reserved OSD strips. Pure.
/// GL viewports start at the bottom-left; ratios run in `f64`, narrowing once.
#[cfg(test)]
pub(crate) fn fit_viewport(
    surface_w: i32,
    surface_h: i32,
    strip: nerust_render_traits::ReservedStrip,
    logical_w: i32,
    logical_h: i32,
) -> Result<([i32; 4], [f32; 16]), FrameError> {
    fit_viewport_aspect(
        surface_w,
        surface_h,
        strip,
        (logical_w as f64, logical_h as f64),
    )
}

pub(crate) fn fit_viewport_aspect(
    surface_w: i32,
    surface_h: i32,
    strip: nerust_render_traits::ReservedStrip,
    physical: (f64, f64),
) -> Result<([i32; 4], [f32; 16]), FrameError> {
    if surface_w <= 0 || surface_h <= 0 {
        return Err(FrameError::InvalidViewport {
            w: surface_w,
            h: surface_h,
        });
    }
    if physical.0 <= 0.0 || physical.1 <= 0.0 || !physical.0.is_finite() || !physical.1.is_finite()
    {
        return Err(FrameError::InvalidViewport { w: 0, h: 0 });
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
    let content_aspect = physical.0 / physical.1;
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
    fn shared_validation_preserves_gl_error_mapping_and_geometry() {
        use nerust_render_traits::logical::LogicalSize;
        let size = LogicalSize {
            width: 2,
            height: 2,
        };
        for unpack in [true, false] {
            for (size, bpp, stride, have, expected) in [
                (size, 0, 12, 32, FrameError::DimensionsOverflow { have: 32 }),
                (
                    LogicalSize { width: 0, ..size },
                    4,
                    12,
                    32,
                    FrameError::DimensionsOverflow { have: 32 },
                ),
                (
                    LogicalSize {
                        width: usize::MAX,
                        ..size
                    },
                    4,
                    12,
                    32,
                    FrameError::DimensionsOverflow { have: 32 },
                ),
                (size, 4, 4, 32, FrameError::StrideOverflow { have: 32 }),
                (size, 4, 9, 32, FrameError::StrideOverflow { have: 32 }),
                (
                    size,
                    1,
                    usize::MAX,
                    usize::MAX,
                    FrameError::StrideOverflow { have: usize::MAX },
                ),
                (
                    size,
                    4,
                    12,
                    19,
                    FrameError::Shared(nerust_render_traits::FrameError::TooShort {
                        need: 20,
                        have: 19,
                    }),
                ),
            ] {
                assert_eq!(
                    super::FrameLayout::checked(size, bpp, stride, have, unpack).err(),
                    Some(expected)
                );
            }
            for have in [20, 99] {
                let shared = nerust_render_traits::FrameLayout::checked(size, 4, 12, have).unwrap();
                let gl = super::FrameLayout::checked(size, 4, 12, have, unpack).unwrap();
                assert_eq!((gl.width, gl.height), (size.width, size.height));
                assert_eq!(gl.row_bytes, shared.row_bytes());
                assert_eq!(gl.stride, shared.stride());
                assert_eq!(gl.need, shared.required_bytes());
                assert_eq!(gl.stride_px, if unpack { 3 } else { 2 });
            }
        }
    }

    #[test]
    fn gl_limits_dimensions_on_every_path_but_stride_only_for_unpack() {
        use nerust_render_traits::logical::LogicalSize;
        let too_wide = i32::MAX as usize + 1;
        for unpack in [true, false] {
            for size in [
                LogicalSize {
                    width: too_wide,
                    height: 1,
                },
                LogicalSize {
                    width: 1,
                    height: too_wide,
                },
            ] {
                assert_eq!(
                    super::FrameLayout::checked(size, 1, size.width, usize::MAX, unpack).err(),
                    Some(FrameError::StrideTooWide { have: usize::MAX })
                );
            }
        }
        let size = LogicalSize {
            width: 1,
            height: 1,
        };
        assert_eq!(
            super::FrameLayout::checked(size, 1, too_wide, 1, true).err(),
            Some(FrameError::StrideTooWide { have: 1 })
        );
        let packed = super::FrameLayout::checked(size, 1, too_wide, 1, false).unwrap();
        assert_eq!((packed.stride_px, packed.need), (1, 1));
    }

    #[test]
    fn frame_layout_rejects_overlapping_rows_on_every_path() {
        let size = nerust_render_traits::logical::LogicalSize {
            width: 4,
            height: 2,
        };
        for unpack in [true, false] {
            for stride in [0, 4, 12] {
                assert!(super::FrameLayout::checked(size, 4, stride, 64, unpack).is_err());
            }
        }
    }

    #[test]
    fn frame_layout_accepts_an_unpadded_final_row_on_every_path() {
        let size = nerust_render_traits::logical::LogicalSize {
            width: 2,
            height: 2,
        };
        for unpack in [true, false] {
            let layout = super::FrameLayout::checked(size, 4, 12, 20, unpack).unwrap();
            assert_eq!(layout.need, 20);
        }
    }

    #[test]
    fn viewport_uses_fractional_physical_aspect() {
        let (_, matrix) =
            super::fit_viewport_aspect(400, 300, Default::default(), (320.5, 240.375)).unwrap();
        assert_eq!(matrix[0], 1.0);
        assert_eq!(matrix[5], 1.0);
        assert!(
            super::fit_viewport_aspect(400, 300, Default::default(), (f64::NAN, 240.0)).is_err()
        );
    }

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
            FrameError::Shared(nerust_render_traits::FrameError::TooShort { need: 8, have: 2 })
                .to_string(),
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
            Err(FrameError::Shared(
                nerust_render_traits::FrameError::TooShort { .. }
            ))
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
