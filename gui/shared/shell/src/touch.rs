use nerust_input_nes::topology::{
    NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT,
    NES_CONTROL_RIGHT, NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::DigitalInputEvent;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl TouchRect {
    fn contains(self, point: TouchPoint) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchOverlayAction {
    Input(DigitalInputEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchTarget {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Start,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchZone {
    pub target: TouchTarget,
    pub bounds: TouchRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortraitTouchOverlay {
    zones: Vec<TouchZone>,
}

impl PortraitTouchOverlay {
    pub fn new(width: f32, height: f32) -> Self {
        let control_top = height * 0.54;
        let control_height = height - control_top;
        let dpad_left = width * 0.08;
        let dpad_size = width * 0.28;
        let dpad_center_x = dpad_left + dpad_size * 0.50;
        let dpad_center_y = control_top + control_height * 0.58;
        let dpad_arm = dpad_size * 0.28;
        let dpad_extent = dpad_size * 0.38;
        let action_size = width * 0.14;
        let action_gap = width * 0.04;
        let action_left = width * 0.64;
        let action_top = dpad_center_y - action_size * 0.50;
        let center_button_width = width * 0.10;
        let center_button_height = height * 0.038;
        let center_gap = width * 0.03;
        let center_row_width = center_button_width * 2.0 + center_gap;
        let center_left_bound = dpad_left + dpad_size + width * 0.03;
        let center_right_bound = action_left - width * 0.03;
        let centered_start = (center_left_bound + center_right_bound - center_row_width) * 0.5;
        let center_start_x = centered_start
            .max(center_left_bound)
            .min(center_left_bound.max(center_right_bound - center_row_width));
        let center_top = control_top + control_height * 0.16;

        let zones = vec![
            TouchZone {
                target: TouchTarget::Up,
                bounds: TouchRect {
                    x: dpad_center_x - dpad_arm * 0.5,
                    y: dpad_center_y - dpad_extent,
                    width: dpad_arm,
                    height: dpad_extent - dpad_arm * 0.5,
                },
            },
            TouchZone {
                target: TouchTarget::Down,
                bounds: TouchRect {
                    x: dpad_center_x - dpad_arm * 0.5,
                    y: dpad_center_y + dpad_arm * 0.5,
                    width: dpad_arm,
                    height: dpad_extent - dpad_arm * 0.5,
                },
            },
            TouchZone {
                target: TouchTarget::Left,
                bounds: TouchRect {
                    x: dpad_center_x - dpad_extent,
                    y: dpad_center_y - dpad_arm * 0.5,
                    width: dpad_extent - dpad_arm * 0.5,
                    height: dpad_arm,
                },
            },
            TouchZone {
                target: TouchTarget::Right,
                bounds: TouchRect {
                    x: dpad_center_x + dpad_arm * 0.5,
                    y: dpad_center_y - dpad_arm * 0.5,
                    width: dpad_extent - dpad_arm * 0.5,
                    height: dpad_arm,
                },
            },
            TouchZone {
                target: TouchTarget::B,
                bounds: TouchRect {
                    x: action_left,
                    y: action_top,
                    width: action_size,
                    height: action_size,
                },
            },
            TouchZone {
                target: TouchTarget::A,
                bounds: TouchRect {
                    x: action_left + action_size + action_gap,
                    y: action_top,
                    width: action_size,
                    height: action_size,
                },
            },
            TouchZone {
                target: TouchTarget::Select,
                bounds: TouchRect {
                    x: center_start_x,
                    y: center_top,
                    width: center_button_width,
                    height: center_button_height,
                },
            },
            TouchZone {
                target: TouchTarget::Start,
                bounds: TouchRect {
                    x: center_start_x + center_button_width + center_gap,
                    y: center_top,
                    width: center_button_width,
                    height: center_button_height,
                },
            },
        ];

        Self { zones }
    }

    pub fn zones(&self) -> &[TouchZone] {
        &self.zones
    }

    pub fn hit_test(&self, point: TouchPoint) -> Option<TouchTarget> {
        self.zones
            .iter()
            .find(|zone| zone.bounds.contains(point))
            .map(|zone| zone.target)
    }
}

pub fn actions_for_target(target: TouchTarget, pressed: bool) -> Vec<TouchOverlayAction> {
    let input = |control| {
        if pressed {
            TouchOverlayAction::Input(DigitalInputEvent::pressed(
                NES_ATTACHMENT_PLAYER_ONE,
                control,
            ))
        } else {
            TouchOverlayAction::Input(DigitalInputEvent::released(
                NES_ATTACHMENT_PLAYER_ONE,
                control,
            ))
        }
    };

    match target {
        TouchTarget::Up => vec![input(NES_CONTROL_UP)],
        TouchTarget::Down => vec![input(NES_CONTROL_DOWN)],
        TouchTarget::Left => vec![input(NES_CONTROL_LEFT)],
        TouchTarget::Right => vec![input(NES_CONTROL_RIGHT)],
        TouchTarget::A => vec![input(NES_CONTROL_A)],
        TouchTarget::B => vec![input(NES_CONTROL_B)],
        TouchTarget::Start => vec![input(NES_CONTROL_START)],
        TouchTarget::Select => vec![input(NES_CONTROL_SELECT)],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PortraitTouchOverlay, TouchOverlayAction, TouchPoint, TouchRect, TouchTarget,
        actions_for_target,
    };
    use nerust_input_nes::topology::{
        NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_LEFT, NES_CONTROL_UP,
    };
    use nerust_input_schema::DigitalInputEvent;

    fn zone_center(bounds: TouchRect) -> TouchPoint {
        TouchPoint {
            x: bounds.x + bounds.width / 2.0,
            y: bounds.y + bounds.height / 2.0,
        }
    }

    fn bounds_for_target(overlay: &PortraitTouchOverlay, target: TouchTarget) -> TouchRect {
        overlay
            .zones()
            .iter()
            .find(|zone| zone.target == target)
            .map(|zone| zone.bounds)
            .expect("zone should exist")
    }

    #[test]
    fn portrait_layout_maps_points_to_expected_targets() {
        let overlay = PortraitTouchOverlay::new(1080.0, 1920.0);
        let up_bounds = bounds_for_target(&overlay, TouchTarget::Up);
        let a_bounds = bounds_for_target(&overlay, TouchTarget::A);

        assert_eq!(
            overlay.hit_test(zone_center(up_bounds)),
            Some(TouchTarget::Up)
        );
        assert_eq!(
            overlay.hit_test(zone_center(a_bounds)),
            Some(TouchTarget::A)
        );
        assert_eq!(overlay.hit_test(TouchPoint { x: 50.0, y: 200.0 }), None);
        assert_eq!(
            overlay.hit_test(TouchPoint {
                x: 120.0,
                y: 1760.0
            }),
            None
        );
    }

    #[test]
    fn button_targets_translate_to_pressed_and_released_input() {
        assert_eq!(
            actions_for_target(TouchTarget::A, true),
            vec![TouchOverlayAction::Input(DigitalInputEvent::pressed(
                NES_ATTACHMENT_PLAYER_ONE,
                NES_CONTROL_A
            ))]
        );
        assert_eq!(
            actions_for_target(TouchTarget::Left, false),
            vec![TouchOverlayAction::Input(DigitalInputEvent::released(
                NES_ATTACHMENT_PLAYER_ONE,
                NES_CONTROL_LEFT
            ))]
        );
        assert_eq!(
            actions_for_target(TouchTarget::Up, true),
            vec![TouchOverlayAction::Input(DigitalInputEvent::pressed(
                NES_ATTACHMENT_PLAYER_ONE,
                NES_CONTROL_UP
            ))]
        );
    }

    #[test]
    fn portrait_overlay_exposes_only_gamepad_zones() {
        let overlay = PortraitTouchOverlay::new(1080.0, 1920.0);
        let zones = overlay.zones();
        assert_eq!(zones.len(), 8);
    }

    #[test]
    fn portrait_overlay_keeps_dpad_and_face_buttons_aligned() {
        let overlay = PortraitTouchOverlay::new(1080.0, 1920.0);
        let up = bounds_for_target(&overlay, TouchTarget::Up);
        let down = bounds_for_target(&overlay, TouchTarget::Down);
        let left = bounds_for_target(&overlay, TouchTarget::Left);
        let right = bounds_for_target(&overlay, TouchTarget::Right);
        let select = bounds_for_target(&overlay, TouchTarget::Select);
        let start = bounds_for_target(&overlay, TouchTarget::Start);
        let b = bounds_for_target(&overlay, TouchTarget::B);
        let a = bounds_for_target(&overlay, TouchTarget::A);

        assert_eq!(up.x, down.x);
        assert_eq!(up.width, down.width);
        assert_eq!(left.y, right.y);
        assert_eq!(left.height, right.height);
        assert_eq!(a.y, b.y);
        assert_eq!(a.height, b.height);
        assert!(b.x > right.x + right.width);
        assert!(a.x > b.x + b.width);
        assert!(select.y + select.height < b.y);
        assert!(start.y + start.height < a.y);
        assert!(select.x > left.x + left.width);
        assert!(start.x + start.width < b.x);
    }
}
