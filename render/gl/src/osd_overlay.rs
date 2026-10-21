//! OSD overlay domain: validation, retry decisions, and GL row addressing. The retry budget is the
//! shared `UploadBudget`; this module only maps its boolean outcome to the overlay decision.

use nerust_render_traits::renderer::OsdLayer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    Retry,
    DropLayer,
}

impl RetryDecision {
    /// Map one shared-policy outcome: `true` (give up) drops the layer.
    pub(crate) fn from_policy(drop: bool) -> Self {
        if drop { Self::DropLayer } else { Self::Retry }
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
    if layer.rgba().len() < need {
        return Err(OsdReject::Short {
            need,
            have: layer.rgba().len(),
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

/// Checked top-first texture row. Our quad samples v=0 at its top, so
/// framebuffer bottom-left addressing must not be applied to texture uploads.
pub(crate) fn checked_osd_y(surface_h: i32, ly: i32, lh: i32) -> Result<i32, OsdReject> {
    if surface_h <= 0 || ly < 0 || lh <= 0 {
        return Err(OsdReject::OutOfBounds);
    }
    let bottom = ly.checked_add(lh).ok_or(OsdReject::OutOfBounds)?;
    if bottom > surface_h {
        return Err(OsdReject::OutOfBounds);
    }
    Ok(ly)
}

/// Checked OSD scratch need: `w > 0 && h > 0`, then checked `w * h * 4`.
/// Rejects before any scratch resize so a corrupt rect never OOMs the thread.
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

/// Pure upload policy and geometry validation.
#[cfg(test)]
mod tests {
    use super::{OsdReject, RetryDecision, checked_clear_need, checked_osd_rect, checked_osd_y};
    use crate::upload::BoundedUploadPolicy;
    use nerust_render_traits::renderer::{OsdLayer, OsdUploadPolicy};

    #[test]
    fn decision_maps_shared_policy_outcome() {
        assert_eq!(RetryDecision::from_policy(false), RetryDecision::Retry);
        assert_eq!(RetryDecision::from_policy(true), RetryDecision::DropLayer);
        // End to end through the single budget: two retries, then drop.
        let mut budget = BoundedUploadPolicy::new(3);
        assert_eq!(
            RetryDecision::from_policy(budget.record_failure()),
            RetryDecision::Retry
        );
        assert_eq!(
            RetryDecision::from_policy(budget.record_failure()),
            RetryDecision::Retry
        );
        assert_eq!(
            RetryDecision::from_policy(budget.record_failure()),
            RetryDecision::DropLayer
        );
        assert_eq!(
            RetryDecision::from_policy(budget.record_failure()),
            RetryDecision::DropLayer
        );
        budget.reset();
        assert_eq!(
            RetryDecision::from_policy(budget.record_failure()),
            RetryDecision::Retry
        );
    }

    #[test]
    fn texture_rows_match_top_first_uvs() {
        assert_eq!(checked_osd_y(64, 6, 1), Ok(6));
        assert_eq!(checked_osd_y(64, 0, 1), Ok(0));
        assert_eq!(checked_osd_y(64, 63, 1), Ok(63));
    }

    #[test]
    fn checked_rows_reject_bad_geometry() {
        assert_eq!(checked_osd_y(64, 6, 1), Ok(6));
        assert_eq!(checked_osd_y(64, 0, 1), Ok(0));
        assert_eq!(checked_osd_y(64, 63, 1), Ok(63));
        assert_eq!(checked_osd_y(64, 64, 1), Err(OsdReject::OutOfBounds));
        assert_eq!(checked_osd_y(64, 0, 65), Err(OsdReject::OutOfBounds));
        assert_eq!(checked_osd_y(64, -1, 1), Err(OsdReject::OutOfBounds));
        assert_eq!(
            checked_osd_y(i32::MAX, i32::MAX, 1),
            Err(OsdReject::OutOfBounds)
        );
    }

    #[test]
    fn checked_math_rejects_overflow_and_wrap() {
        let huge = OsdLayer::new(0, 0, u32::MAX, u32::MAX, vec![]);
        assert_eq!(
            checked_osd_rect(&huge, i32::MAX, i32::MAX),
            Err(OsdReject::Empty)
        );
        // lx + lw overflow refuses instead of wrapping.
        let edge = OsdLayer::new(u32::MAX, 0, 2, 1, vec![0; 8]);
        assert_eq!(
            checked_osd_rect(&edge, i32::MAX, i32::MAX),
            Err(OsdReject::OutOfBounds)
        );
        // clear need rejects zero and overflow.
        assert_eq!(checked_clear_need(0, 4), Err(OsdReject::Empty));
        assert_eq!(checked_clear_need(-1, 4), Err(OsdReject::Empty));
        // `i32::MAX` fits `usize` on 64-bit: the checked chain returns the
        // huge need instead of wrapping; the check proves no truncation.
        let huge = checked_clear_need(i32::MAX, i32::MAX).unwrap();
        assert_eq!(huge, 18_446_744_056_529_682_436usize);
        assert!(checked_clear_need(2, 1).unwrap() == 8);
    }

    #[test]
    fn checked_rect_rejects_negative_and_zero_geometry() {
        use nerust_render_traits::renderer::OsdLayer;
        // Negative origin refuses instead of wrapping.
        assert_eq!(
            checked_osd_rect(&OsdLayer::new(0, 0, 0, 1, vec![]), 64, 64),
            Err(OsdReject::Empty)
        );
        // Layer past the surface edge refuses.
        assert_eq!(
            checked_osd_rect(&OsdLayer::new(63, 63, 2, 2, vec![0; 16]), 64, 64),
            Err(OsdReject::OutOfBounds)
        );
        assert_eq!(
            checked_osd_rect(&OsdLayer::new(0, 0, 2, 1, vec![0; 8]), 1, 64),
            Err(OsdReject::OutOfBounds)
        );
    }

    #[test]
    fn checked_rows_reject_empty_surface() {
        assert_eq!(checked_osd_y(0, 0, 1), Err(OsdReject::OutOfBounds));
        assert_eq!(checked_osd_y(64, 0, 0), Err(OsdReject::OutOfBounds));
    }
}
