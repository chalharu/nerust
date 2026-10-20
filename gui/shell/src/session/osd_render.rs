//! Render adapter: the single point where OSD decisions cross into render vocabulary.
//! Only this module names `nerust_render_traits` types and maps settings corners to anchors.

use nerust_render_traits::renderer::{OsdAnchor, SafeInsets};
use nerust_settings_core::model::local::NotificationPosition;

use crate::session::osd_presenter::{OsdPlan, UiInsets};

/// Status pill color, identical on every frontend; transient rows keep per-level colors.
pub const OSD_STATUS_RGB: (u8, u8, u8) = (200, 200, 200);

/// Settings corner mapped once to the render-space anchor.
/// The rasterizer only speaks [`OsdAnchor`]; this is the single crossing point.
pub fn osd_anchor(position: NotificationPosition) -> OsdAnchor {
    match position {
        NotificationPosition::TopLeft => OsdAnchor::TopLeft,
        NotificationPosition::TopRight => OsdAnchor::TopRight,
        NotificationPosition::BottomLeft => OsdAnchor::BottomLeft,
        NotificationPosition::BottomRight => OsdAnchor::BottomRight,
    }
}

impl From<SafeInsets> for UiInsets {
    fn from(insets: SafeInsets) -> Self {
        Self {
            top: insets.top,
            right: insets.right,
            bottom: insets.bottom,
            left: insets.left,
        }
    }
}

impl OsdPlan {
    /// Single construction shared by every frontend: rows plus the status pill.
    /// Takes ownership; row tags ride through as static markers with no per-frame allocation.
    pub fn into_content(self) -> nerust_render_traits::renderer::OsdContent {
        use nerust_render_traits::renderer::{OsdContent, OsdRow, OsdStatus};
        OsdContent {
            rows: self
                .rows
                .into_iter()
                .map(|row| OsdRow {
                    tag: row.tag,
                    text: row.text,
                    rgb: row.rgb,
                })
                .collect(),
            status: self.status.map(|(label, value)| OsdStatus {
                label,
                value,
                rgb: OSD_STATUS_RGB,
            }),
            position: osd_anchor(self.position),
            status_position: osd_anchor(self.status_position),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::osd_anchor;
    use nerust_render_traits::renderer::OsdAnchor as A;
    use nerust_settings_core::model::local::NotificationPosition as P;

    #[test]
    fn anchor_mapping_covers_all_corners() {
        assert_eq!(osd_anchor(P::TopLeft), A::TopLeft);
        assert_eq!(osd_anchor(P::TopRight), A::TopRight);
        assert_eq!(osd_anchor(P::BottomLeft), A::BottomLeft);
        assert_eq!(osd_anchor(P::BottomRight), A::BottomRight);
    }
}
