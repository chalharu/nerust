//! Render adapter: the single point where OSD decisions cross into render vocabulary.
//! Only this module names `nerust_render_traits` types and maps settings corners to anchors.

use nerust_render_traits::renderer::{OsdAnchor, OsdContent, OsdRow, OsdStatus};
use nerust_settings_core::model::local::NotificationPosition;

use crate::session::osd_presenter::OsdPlan;

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

impl OsdPlan {
    /// Single construction shared by every frontend: rows plus the status pill.
    /// Takes ownership; row tags ride through as shared handles with no per-frame allocation.
    pub fn into_content(self) -> OsdContent {
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

#[cfg(test)]
mod render_cov_tests {
    use crate::session::osd_render::*;
    use std::sync::Arc;

    #[test]
    fn plan_into_content_carries_status_color() {
        use crate::session::osd_presenter::OsdPlan;
        use nerust_settings_core::model::local::NotificationPosition as P;
        let plan = OsdPlan {
            rows: vec![],
            status: Some(("FPS".into(), "60".into())),
            position: P::TopLeft,
            status_position: P::BottomRight,
            width: 0,
            height: 0,
        };
        let content = plan.into_content();
        assert!(content.rows.is_empty());
        let status = content.status.expect("status rides through");
        assert_eq!(status.rgb, OSD_STATUS_RGB);
    }

    #[test]
    fn plan_without_status_maps_rows() {
        use crate::session::osd_presenter::OsdPlan;
        use crate::session::osd_transient::OsdRow;
        use nerust_settings_core::model::local::NotificationPosition as P;
        let plan = OsdPlan {
            rows: vec![OsdRow {
                text: Arc::from("Paused"),
                rgb: (1, 2, 3),
                tag: "[INFO]".into(),
            }],
            status: None,
            position: P::TopLeft,
            status_position: P::TopLeft,
            width: 0,
            height: 0,
        };
        let content = plan.into_content();
        assert_eq!(content.rows.len(), 1);
        assert!(content.status.is_none());
    }
}
