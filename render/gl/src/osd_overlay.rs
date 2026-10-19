//! OSD overlay domain: validation, retry, and GL row addressing.
//! Pure geometry lives here so the corrupt-layer policy stays
//! unit-testable; [`OsdOverlay`] owns the retry budget plus the
//! grow-only staging scratches.

use nerust_render_traits::renderer::OsdLayer;

/// Bounded OSD retry: transient blips survive, dead drivers drop.
/// Pure counter; the renderer owns warn-once logging.
#[derive(Debug)]
pub(crate) struct RetryBudget {
    pub(crate) max: u8,
    pub(crate) fails: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    Retry,
    DropLayer,
}

impl RetryBudget {
    pub(crate) fn new(max: u8) -> Self {
        Self { max, fails: 0 }
    }

    pub(crate) fn record_failure(&mut self) -> RetryDecision {
        self.fails = self.fails.saturating_add(1);
        if self.fails >= self.max {
            RetryDecision::DropLayer
        } else {
            RetryDecision::Retry
        }
    }

    pub(crate) fn reset(&mut self) {
        self.fails = 0;
    }
}

/// Why an OSD layer was rejected. Logged at the drop site so a
/// corrupt layer names its fault instead of a bare "invalid".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OsdReject {
    Empty,
    Short { need: usize, have: usize },
    OutOfBounds,
}

/// Validated OSD blit geometry. Pure so the corrupt-layer policy
/// stays unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OSDRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub stride: usize,
    pub need: usize,
}

pub(crate) fn checked_osd_rect(
    layer: &OsdLayer,
    surface_w: i32,
    surface_h: i32,
) -> Result<OSDRect, OsdReject> {
    let (w, h) = (
        usize::try_from(layer.width).map_err(|_| OsdReject::Empty)?,
        usize::try_from(layer.height).map_err(|_| OsdReject::Empty)?,
    );
    let stride = w.checked_mul(4).ok_or(OsdReject::Empty)?;
    let need = stride.checked_mul(h).ok_or(OsdReject::Empty)?;
    if need == 0 {
        return Err(OsdReject::Empty);
    }
    if layer.rgba.len() < need {
        return Err(OsdReject::Short {
            need,
            have: layer.rgba.len(),
        });
    }
    // u32 -> i32 via try_from: values above i32::MAX refuse as
    // out-of-bounds instead of wrapping negative.
    let lx = i32::try_from(layer.x).map_err(|_| OsdReject::OutOfBounds)?;
    let ly = i32::try_from(layer.y).map_err(|_| OsdReject::OutOfBounds)?;
    let lw = i32::try_from(w).map_err(|_| OsdReject::OutOfBounds)?;
    let lh = i32::try_from(h).map_err(|_| OsdReject::OutOfBounds)?;
    if lx < 0 || ly < 0 {
        return Err(OsdReject::OutOfBounds);
    }
    let right = (i64::from(lx))
        .checked_add(i64::from(lw))
        .ok_or(OsdReject::OutOfBounds)?;
    let bottom = (i64::from(ly))
        .checked_add(i64::from(lh))
        .ok_or(OsdReject::OutOfBounds)?;
    if right > i64::from(surface_w) || bottom > i64::from(surface_h) {
        return Err(OsdReject::OutOfBounds);
    }
    Ok(OSDRect {
        x: lx,
        y: ly,
        w: lw,
        h: lh,
        stride,
        need,
    })
}

/// GL textures start at the bottom-left; OSD layers start at the
/// top-left. Flip once at the upload edge so blit and clear agree.
pub(crate) fn osd_gl_y(surface_h: i32, ly: i32, lh: i32) -> i32 {
    surface_h - ly - lh
}

/// Checked OSD scratch need: `w > 0 && h > 0`, then checked
/// `w * h * 4`. Rejects before any scratch resize so a corrupt
/// rect never OOMs the renderer thread.
pub(crate) fn checked_clear_need(w: i32, h: i32) -> Result<usize, OsdReject> {
    if w <= 0 || h <= 0 {
        return Err(OsdReject::Empty);
    }
    let w = usize::try_from(w).map_err(|_| OsdReject::Empty)?;
    let h = usize::try_from(h).map_err(|_| OsdReject::Empty)?;
    w.checked_mul(h)
        .and_then(|px| px.checked_mul(4))
        .ok_or(OsdReject::Empty)
}

/// Overlay staging domain note: the live owner is `OsdResources`
/// in `view.rs` (retry budget + both grow-only scratches + last
/// rect); this module owns the pure policy (`RetryBudget`,
/// `checked_osd_rect`, `osd_gl_y`, `checked_clear_need`) so `GlView`
/// only coordinates uploads.
#[cfg(test)]
mod tests {
    use super::{OsdReject, checked_clear_need, checked_osd_rect, osd_gl_y};
    use nerust_render_traits::renderer::OsdLayer;

    #[test]
    fn y_flip_matches_bottom_left_origin() {
        // 64px surface, 2x1 layer at top-left rows: GL row = 64-6-1.
        assert_eq!(osd_gl_y(64, 6, 1), 57);
        assert_eq!(osd_gl_y(64, 0, 1), 63);
        assert_eq!(osd_gl_y(64, 63, 1), 0);
    }

    #[test]
    fn checked_math_rejects_overflow_and_wrap() {
        let huge = OsdLayer {
            x: 0,
            y: 0,
            width: u32::MAX,
            height: u32::MAX,
            rgba: vec![],
        };
        assert_eq!(
            checked_osd_rect(&huge, i32::MAX, i32::MAX),
            Err(OsdReject::Empty)
        );
        // lx + lw overflow refuses instead of wrapping.
        let edge = OsdLayer {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert_eq!(
            checked_osd_rect(&edge, i32::MAX, i32::MAX),
            Err(OsdReject::OutOfBounds)
        );
        // clear need rejects zero and overflow.
        assert_eq!(checked_clear_need(0, 4), Err(OsdReject::Empty));
        assert_eq!(checked_clear_need(-1, 4), Err(OsdReject::Empty));
        // i32::MAX dimensions fit `usize` on 64-bit (no wrap to small):
        // the checked chain returns the huge need instead of wrapping.
        // Callers only reach here with surface-bounded rects, so no
        // 18 EiB scratch ever allocates; the check proves no truncation.
        let huge = checked_clear_need(i32::MAX, i32::MAX).unwrap();
        assert_eq!(huge, 18_446_744_056_529_682_436usize);
        assert!(checked_clear_need(2, 1).unwrap() == 8);
    }
}
