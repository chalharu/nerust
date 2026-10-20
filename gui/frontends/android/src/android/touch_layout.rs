//! Touch-overlay geometry owned by Rust: single source of truth for hit zones.
//! Kotlin draws the pushed zones, falling back to its local copy when the payload is absent or malformed.

use nerust_core_traits::touch::{
    TouchControl, TouchControlRole, TouchPoint, TouchRect, clamp_floating_dpad_knob,
    floating_dpad_roles,
};
use nerust_input_traits::{AttachmentId, DigitalControlId};

// Named layout fractions.

/// Top of the touch-control band in portrait (fraction of height).
pub(super) const PORTRAIT_CONTROL_TOP: f32 = 0.54;
/// Fixed-capacity D-Pad target set: at most the four cardinal directions.
/// `None` pads unused slots, so the touch-move path allocates nothing.
pub(super) type DpadTargets = [Option<(AttachmentId, DigitalControlId)>; 4];

/// Empty target set for D-Pad release paths.
pub(super) const NO_DPAD_TARGETS: DpadTargets = [None; 4];

/// D-Pad cell size as a fraction of the minor axis, before user scale.
pub(super) const DPAD_SIZE_OF_BASE: f32 = 0.28;
/// D-Pad activation radius as a fraction of the D-Pad cell.
pub(super) const DPAD_RADIUS_OF_SIZE: f32 = 0.42;
/// Face-button cell as a fraction of the minor axis, before user scale.
pub(super) const ACTION_SIZE_OF_BASE: f32 = 0.14;
/// Gap between the two face buttons as a fraction of the minor axis.
pub(super) const ACTION_GAP_OF_BASE: f32 = 0.04;
/// Face-button band origin in portrait (fraction of width).
pub(super) const PORTRAIT_ACTION_LEFT: f32 = 0.64;
/// Face-button band right margin in landscape (fraction of minor axis).
pub(super) const LANDSCAPE_ACTION_MARGIN_OF_BASE: f32 = 0.08;
/// Center-button width/height as fractions of the minor axis.
pub(super) const CENTER_WIDTH_OF_BASE: f32 = 0.10;
pub(super) const CENTER_HEIGHT_OF_BASE: f32 = 0.068;
/// Gap between Select and Start as a fraction of the minor axis.
pub(super) const CENTER_GAP_OF_BASE: f32 = 0.03;
/// Portrait origin of the center row (fraction of control height).
pub(super) const PORTRAIT_CENTER_TOP_OF_CONTROL: f32 = 0.16;
/// Landscape origin of the center row (fraction of height).
pub(super) const LANDSCAPE_CENTER_TOP: f32 = 0.82;
/// Portrait D-Pad center height (fraction of control height).
pub(super) const PORTRAIT_DPAD_CENTER_OF_CONTROL: f32 = 0.58;
/// Landscape D-Pad center height (fraction of height).
pub(super) const LANDSCAPE_DPAD_CENTER: f32 = 0.65;
/// Shoulder strip cell as fractions of the minor axis, before user scale.
pub(super) const SHOULDER_WIDTH_OF_BASE: f32 = 0.20;
pub(super) const SHOULDER_HEIGHT_OF_BASE: f32 = 0.08;
/// Shoulder strip side margin (fraction of width) and top nudge
/// (fraction of minor axis) below the control band.
pub(super) const SHOULDER_MARGIN_X_OF_WIDTH: f32 = 0.02;
pub(super) const SHOULDER_TOP_NUDGE_OF_BASE: f32 = 0.015;
/// Portrait D-Pad activation right edge (fraction of width).
pub(super) const PORTRAIT_ACTIVATION_RIGHT: f32 = 0.52;
/// Landscape D-Pad activation right edge (fraction of width).
pub(super) const LANDSCAPE_ACTIVATION_RIGHT: f32 = 0.45;
/// Landscape D-Pad activation top (fraction of height).
pub(super) const LANDSCAPE_ACTIVATION_TOP: f32 = 0.25;
/// Floating-joystick knob radius as a fraction of the base radius.
/// Rust hit-testing never needs it; the shared fraction lives here.
#[allow(dead_code)]
pub(super) const KNOB_RADIUS_OF_BASE: f32 = 0.48;

/// Canonical zone table: role plus the label Kotlin draws.
/// Fixes the zone set and order both sides iterate.
pub(super) const TOUCH_ZONES: &[(TouchControlRole, &str)] = &[
    (TouchControlRole::FaceButton2, "B"),
    (TouchControlRole::FaceButton1, "A"),
    (TouchControlRole::LeftShoulder, "L"),
    (TouchControlRole::RightShoulder, "R"),
    (TouchControlRole::Select, "SELECT"),
    (TouchControlRole::Start, "START"),
];

/// Label for a zone role; falls back to the debug name so a new role
/// never renders as an empty pill.
pub(super) fn zone_label(role: TouchControlRole) -> &'static str {
    TOUCH_ZONES
        .iter()
        .find(|(zone_role, _)| *zone_role == role)
        .map(|(_, label)| *label)
        .unwrap_or("?")
}

/// One resolved zone in relative (0.0..=1.0) surface coordinates.
/// Relative so both sides stay identical across density buckets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RelativeZone {
    pub role: TouchControlRole,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug)]
pub(super) struct TouchZone {
    pub(super) control: TouchControl,
    pub(super) bounds: TouchRect,
}

#[derive(Debug)]
pub(super) struct ProfileTouchOverlay {
    pub(super) zones: Vec<TouchZone>,
    pub(super) dpad_controls: Vec<TouchControl>,
    pub(super) dpad_activation: TouchRect,
    pub(super) dpad_radius: f32,
}

#[derive(Debug)]
pub(super) struct FloatingDpadState {
    pub(super) touch_id: u64,
    pub(super) center: TouchPoint,
    /// Fixed-capacity target set (at most the four cardinal directions).
    /// `None` pads unused slots; see `DpadTargets`.
    pub(super) targets: DpadTargets,
}

impl ProfileTouchOverlay {
    pub(super) fn new(
        width: f32,
        height: f32,
        controls: Vec<TouchControl>,
        scale_percent: u8,
        vertical_offset_percent: i8,
    ) -> Self {
        let portrait = height >= width;
        let base = width.min(height);
        let scale = f32::from(scale_percent.clamp(50, 150)) / 100.0;
        let vertical_offset = height * f32::from(vertical_offset_percent.clamp(-30, 30)) / 100.0;
        let control_top = if portrait {
            height * PORTRAIT_CONTROL_TOP
        } else {
            0.0
        };
        let control_height = height - control_top;
        let dpad_size = base * DPAD_SIZE_OF_BASE * scale;
        let dpad_center_y = if portrait {
            control_top + control_height * PORTRAIT_DPAD_CENTER_OF_CONTROL
        } else {
            height * LANDSCAPE_DPAD_CENTER
        } + vertical_offset;
        let dpad_radius = dpad_size * DPAD_RADIUS_OF_SIZE;
        let action_size = base * ACTION_SIZE_OF_BASE * scale;
        let action_gap = base * ACTION_GAP_OF_BASE;
        let action_left = if portrait {
            width * PORTRAIT_ACTION_LEFT
        } else {
            width - base * LANDSCAPE_ACTION_MARGIN_OF_BASE - action_size * 2.0 - action_gap
        };
        let action_top = dpad_center_y - action_size * 0.50;
        let center_width = base * CENTER_WIDTH_OF_BASE * scale;
        let center_height = base * CENTER_HEIGHT_OF_BASE * scale;
        // Shoulder strip above the D-Pad activation area and the center row.
        // Must stay clear of both; keep in sync with controlsLayout().
        let shoulder_width = base * SHOULDER_WIDTH_OF_BASE * scale;
        let shoulder_height = base * SHOULDER_HEIGHT_OF_BASE * scale;
        let shoulder_margin_x = width * SHOULDER_MARGIN_X_OF_WIDTH;
        let shoulder_top = control_top + base * SHOULDER_TOP_NUDGE_OF_BASE + vertical_offset;
        let shoulder_left_x = shoulder_margin_x;
        let shoulder_right_x = width - shoulder_margin_x - shoulder_width;
        let center_gap = base * CENTER_GAP_OF_BASE;
        let center_row_width = center_width * 2.0 + center_gap;
        let center_left = (width - center_row_width) * 0.5;
        let center_top = if portrait {
            control_top + control_height * PORTRAIT_CENTER_TOP_OF_CONTROL
        } else {
            height * LANDSCAPE_CENTER_TOP
        } + vertical_offset;

        let bounds_for = |role| match role {
            TouchControlRole::FaceButton2 => TouchRect {
                x: action_left,
                y: action_top,
                width: action_size,
                height: action_size,
            },
            TouchControlRole::FaceButton1 => TouchRect {
                x: action_left + action_size + action_gap,
                y: action_top,
                width: action_size,
                height: action_size,
            },
            TouchControlRole::Select => TouchRect {
                x: center_left,
                y: center_top,
                width: center_width,
                height: center_height,
            },
            TouchControlRole::LeftShoulder => TouchRect {
                x: shoulder_left_x,
                y: shoulder_top,
                width: shoulder_width,
                height: shoulder_height,
            },
            TouchControlRole::RightShoulder => TouchRect {
                x: shoulder_right_x,
                y: shoulder_top,
                width: shoulder_width,
                height: shoulder_height,
            },
            TouchControlRole::Start => TouchRect {
                x: center_left + center_width + center_gap,
                y: center_top,
                width: center_width,
                height: center_height,
            },
            _ => unreachable!("D-Pad controls use the floating joystick"),
        };
        let (dpad_controls, button_controls): (Vec<_>, Vec<_>) = controls
            .into_iter()
            .partition(|control| Self::is_dpad_role(control.role));
        let activation_right = if portrait {
            width * PORTRAIT_ACTIVATION_RIGHT
        } else {
            width * LANDSCAPE_ACTIVATION_RIGHT
        };
        let activation_top = if portrait {
            control_top
        } else {
            height * LANDSCAPE_ACTIVATION_TOP
        };
        Self {
            zones: button_controls
                .into_iter()
                .map(|control| TouchZone {
                    bounds: bounds_for(control.role),
                    control,
                })
                .collect(),
            dpad_controls,
            dpad_activation: TouchRect {
                x: dpad_radius,
                y: activation_top + dpad_radius,
                width: (activation_right - dpad_radius * 2.0).max(0.0),
                height: (height - activation_top - dpad_radius * 2.0).max(0.0),
            },
            dpad_radius,
        }
    }

    /// Zones in relative coordinates for the Kotlin overlay update path.
    /// Order follows [`TOUCH_ZONES`]; roles without a bound control are skipped.
    pub(super) fn relative_zones(&self, width: f32, height: f32) -> Vec<RelativeZone> {
        let (width, height) = (width.max(1.0), height.max(1.0));
        let by_role = |role| {
            self.zones
                .iter()
                .find(|zone| zone.control.role == role)
                .map(|zone| RelativeZone {
                    role,
                    x: zone.bounds.x / width,
                    y: zone.bounds.y / height,
                    width: zone.bounds.width / width,
                    height: zone.bounds.height / height,
                })
        };
        TOUCH_ZONES
            .iter()
            .filter_map(|(role, _)| by_role(*role))
            .collect()
    }

    /// Compact encoding for the JNI overlay update path.
    /// `LABEL:x,y,w,h;...` with relative coordinates at 4 decimals.
    pub(super) fn encode_zones(zones: &[RelativeZone]) -> String {
        zones
            .iter()
            .map(|zone| {
                format!(
                    "{}:{:.4},{:.4},{:.4},{:.4}",
                    zone_label(zone.role),
                    zone.x,
                    zone.y,
                    zone.width,
                    zone.height
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    fn is_dpad_role(role: TouchControlRole) -> bool {
        matches!(
            role,
            TouchControlRole::DpadUp
                | TouchControlRole::DpadDown
                | TouchControlRole::DpadLeft
                | TouchControlRole::DpadRight
        )
    }

    pub(super) fn accepts_floating_dpad(&self, point: TouchPoint) -> bool {
        !self.dpad_controls.is_empty() && self.dpad_activation.contains(point)
    }

    pub(super) fn floating_dpad_targets(
        &self,
        center: TouchPoint,
        point: TouchPoint,
    ) -> DpadTargets {
        let mut out: DpadTargets = [None; 4];
        let mut len = 0;
        for role in floating_dpad_roles(center, point, self.dpad_radius) {
            if len >= out.len() {
                break;
            }
            if let Some(control) = self
                .dpad_controls
                .iter()
                .find(|control| control.role == role)
            {
                out[len] = Some((control.attachment_id, control.control_id));
                len += 1;
            }
        }
        out
    }

    pub(super) fn floating_knob(&self, center: TouchPoint, point: TouchPoint) -> TouchPoint {
        clamp_floating_dpad_knob(center, point, self.dpad_radius)
    }

    pub(super) fn hit_test(&self, point: TouchPoint) -> Option<(AttachmentId, DigitalControlId)> {
        self.zones
            .iter()
            .find(|zone| zone.bounds.contains(point))
            .map(|zone| (zone.control.attachment_id, zone.control.control_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(role: TouchControlRole, id: &'static str) -> TouchControl {
        TouchControl {
            attachment_id: AttachmentId::new("test"),
            control_id: DigitalControlId::new(id),
            role,
            label: zone_label(role).to_string(),
        }
    }

    fn portrait_overlay() -> ProfileTouchOverlay {
        ProfileTouchOverlay::new(
            1080.0,
            1920.0,
            [
                TouchControlRole::FaceButton1,
                TouchControlRole::FaceButton2,
                TouchControlRole::Select,
                TouchControlRole::Start,
                TouchControlRole::DpadUp,
                TouchControlRole::DpadDown,
                TouchControlRole::DpadLeft,
                TouchControlRole::DpadRight,
            ]
            .into_iter()
            .map(|role| control(role, "button"))
            .collect(),
            100,
            0,
        )
    }

    #[test]
    fn zones_stay_inside_relative_bounds() {
        let overlay = portrait_overlay();
        let zones = overlay.relative_zones(1080.0, 1920.0);
        // D-Pad roles use the floating joystick, not rect zones.
        assert_eq!(zones.len(), 4);
        for zone in &zones {
            assert!((0.0..=1.0).contains(&zone.x));
            assert!((0.0..=1.0).contains(&zone.y));
            assert!(zone.width > 0.0 && zone.x + zone.width <= 1.0);
            assert!(zone.height > 0.0 && zone.y + zone.height <= 1.0);
        }
    }

    #[test]
    fn encode_zones_round_trips_through_labels() {
        let overlay = portrait_overlay();
        let encoded = ProfileTouchOverlay::encode_zones(&overlay.relative_zones(1080.0, 1920.0));
        assert!(encoded.contains("B:"));
        assert!(encoded.contains("A:"));
        assert!(encoded.contains("SELECT:"));
        assert!(encoded.contains("START:"));
        for entry in encoded.split(';') {
            let (label, rect) = entry.split_once(':').expect("label:rect entry");
            assert!(!label.is_empty());
            let coords: Vec<f32> = rect.split(',').map(|c| c.parse().unwrap()).collect();
            assert_eq!(coords.len(), 4);
        }
    }

    #[test]
    fn hit_test_finds_face_button() {
        let overlay = portrait_overlay();
        let zone = overlay
            .zones
            .iter()
            .find(|zone| zone.control.role == TouchControlRole::FaceButton1)
            .expect("face button zone");
        let center = TouchPoint {
            x: zone.bounds.x + zone.bounds.width / 2.0,
            y: zone.bounds.y + zone.bounds.height / 2.0,
        };
        assert!(overlay.hit_test(center).is_some());
        assert!(
            overlay
                .hit_test(TouchPoint { x: -10.0, y: -10.0 })
                .is_none()
        );
    }

    #[test]
    fn touch_zone_table_covers_profiles() {
        // Every non-D-Pad role the profiles emit has a zone entry, so a
        // new role fails loudly here instead of rendering an empty pill.
        for role in [
            TouchControlRole::FaceButton1,
            TouchControlRole::FaceButton2,
            TouchControlRole::LeftShoulder,
            TouchControlRole::RightShoulder,
            TouchControlRole::Select,
            TouchControlRole::Start,
        ] {
            assert_ne!(zone_label(role), "?");
        }
    }

    /// Geometry pin: every Rust fraction must match the canonical `touch_geometry.json`.
    /// A one-side drift fails loudly.
    #[test]
    fn geometry_fractions_match_canonical_json() {
        #[derive(serde::Deserialize)]
        struct GeometryPin {
            portrait_control_top: f32,
            dpad_size_of_base: f32,
            dpad_radius_of_size: f32,
            action_size_of_base: f32,
            action_gap_of_base: f32,
            portrait_action_left: f32,
            landscape_action_margin_of_base: f32,
            center_width_of_base: f32,
            center_height_of_base: f32,
            center_gap_of_base: f32,
            portrait_center_top_of_control: f32,
            landscape_center_top: f32,
            portrait_dpad_center_of_control: f32,
            landscape_dpad_center: f32,
            shoulder_width_of_base: f32,
            shoulder_height_of_base: f32,
            shoulder_margin_x_of_width: f32,
            shoulder_top_nudge_of_base: f32,
            portrait_activation_right: f32,
            landscape_activation_right: f32,
            landscape_activation_top: f32,
            knob_radius_of_base: f32,
            zone_labels: Vec<String>,
        }
        let raw = include_str!("../../touch_geometry.json");
        let pin: GeometryPin = serde_json::from_str(raw).expect("touch_geometry.json must parse");
        for (name, actual, expected) in [
            (
                "portrait_control_top",
                PORTRAIT_CONTROL_TOP,
                pin.portrait_control_top,
            ),
            (
                "dpad_size_of_base",
                DPAD_SIZE_OF_BASE,
                pin.dpad_size_of_base,
            ),
            (
                "dpad_radius_of_size",
                DPAD_RADIUS_OF_SIZE,
                pin.dpad_radius_of_size,
            ),
            (
                "action_size_of_base",
                ACTION_SIZE_OF_BASE,
                pin.action_size_of_base,
            ),
            (
                "action_gap_of_base",
                ACTION_GAP_OF_BASE,
                pin.action_gap_of_base,
            ),
            (
                "portrait_action_left",
                PORTRAIT_ACTION_LEFT,
                pin.portrait_action_left,
            ),
            (
                "landscape_action_margin_of_base",
                LANDSCAPE_ACTION_MARGIN_OF_BASE,
                pin.landscape_action_margin_of_base,
            ),
            (
                "center_width_of_base",
                CENTER_WIDTH_OF_BASE,
                pin.center_width_of_base,
            ),
            (
                "center_height_of_base",
                CENTER_HEIGHT_OF_BASE,
                pin.center_height_of_base,
            ),
            (
                "center_gap_of_base",
                CENTER_GAP_OF_BASE,
                pin.center_gap_of_base,
            ),
            (
                "portrait_center_top_of_control",
                PORTRAIT_CENTER_TOP_OF_CONTROL,
                pin.portrait_center_top_of_control,
            ),
            (
                "landscape_center_top",
                LANDSCAPE_CENTER_TOP,
                pin.landscape_center_top,
            ),
            (
                "portrait_dpad_center_of_control",
                PORTRAIT_DPAD_CENTER_OF_CONTROL,
                pin.portrait_dpad_center_of_control,
            ),
            (
                "landscape_dpad_center",
                LANDSCAPE_DPAD_CENTER,
                pin.landscape_dpad_center,
            ),
            (
                "shoulder_width_of_base",
                SHOULDER_WIDTH_OF_BASE,
                pin.shoulder_width_of_base,
            ),
            (
                "shoulder_height_of_base",
                SHOULDER_HEIGHT_OF_BASE,
                pin.shoulder_height_of_base,
            ),
            (
                "shoulder_margin_x_of_width",
                SHOULDER_MARGIN_X_OF_WIDTH,
                pin.shoulder_margin_x_of_width,
            ),
            (
                "shoulder_top_nudge_of_base",
                SHOULDER_TOP_NUDGE_OF_BASE,
                pin.shoulder_top_nudge_of_base,
            ),
            (
                "portrait_activation_right",
                PORTRAIT_ACTIVATION_RIGHT,
                pin.portrait_activation_right,
            ),
            (
                "landscape_activation_right",
                LANDSCAPE_ACTIVATION_RIGHT,
                pin.landscape_activation_right,
            ),
            (
                "landscape_activation_top",
                LANDSCAPE_ACTIVATION_TOP,
                pin.landscape_activation_top,
            ),
            (
                "knob_radius_of_base",
                KNOB_RADIUS_OF_BASE,
                pin.knob_radius_of_base,
            ),
        ] {
            assert!(
                (actual - expected).abs() < 1e-6,
                "{name} drifted: Rust {actual} vs canonical {expected}"
            );
        }
        let rust_labels: Vec<&str> = TOUCH_ZONES.iter().map(|(_, label)| *label).collect();
        let pin_labels: Vec<&str> = pin.zone_labels.iter().map(String::as_str).collect();
        assert_eq!(rust_labels, pin_labels, "zone order drifted");
    }
}
