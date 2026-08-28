use nerust_input_traits::{AttachmentId, DigitalControlId, DigitalInputEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchControlRole {
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    FaceButton1,
    FaceButton2,
    Start,
    Select,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchControl {
    pub attachment_id: AttachmentId,
    pub control_id: DigitalControlId,
    pub role: TouchControlRole,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TouchOverlayModel {
    pub revision: u64,
    pub controls: Vec<TouchControl>,
}

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
    pub fn contains(self, point: TouchPoint) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

pub fn floating_dpad_roles(
    center: TouchPoint,
    point: TouchPoint,
    radius: f32,
) -> Vec<TouchControlRole> {
    let delta_x = point.x - center.x;
    let delta_y = point.y - center.y;
    if delta_x.hypot(delta_y) < radius * 0.24 {
        return Vec::new();
    }
    let mut roles = Vec::with_capacity(2);
    if delta_x.abs() >= delta_y.abs() * 0.5 {
        roles.push(if delta_x < 0.0 {
            TouchControlRole::DpadLeft
        } else {
            TouchControlRole::DpadRight
        });
    }
    if delta_y.abs() >= delta_x.abs() * 0.5 {
        roles.push(if delta_y < 0.0 {
            TouchControlRole::DpadUp
        } else {
            TouchControlRole::DpadDown
        });
    }
    roles
}

pub fn clamp_floating_dpad_knob(center: TouchPoint, point: TouchPoint, radius: f32) -> TouchPoint {
    let delta_x = point.x - center.x;
    let delta_y = point.y - center.y;
    let distance = delta_x.hypot(delta_y);
    if distance <= radius || distance == 0.0 {
        return point;
    }
    let scale = radius / distance;
    TouchPoint {
        x: center.x + delta_x * scale,
        y: center.y + delta_y * scale,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchOverlayAction {
    Input(DigitalInputEvent),
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTER: TouchPoint = TouchPoint { x: 100.0, y: 100.0 };

    #[test]
    fn floating_dpad_applies_dead_zone_and_cardinal_directions() {
        assert!(floating_dpad_roles(CENTER, TouchPoint { x: 110.0, y: 100.0 }, 100.0).is_empty());
        assert_eq!(
            floating_dpad_roles(CENTER, TouchPoint { x: 180.0, y: 90.0 }, 100.0),
            vec![TouchControlRole::DpadRight]
        );
    }

    #[test]
    fn floating_dpad_supports_diagonal_input() {
        assert_eq!(
            floating_dpad_roles(CENTER, TouchPoint { x: 180.0, y: 20.0 }, 100.0),
            vec![TouchControlRole::DpadRight, TouchControlRole::DpadUp]
        );
    }

    #[test]
    fn floating_dpad_knob_is_limited_to_base_radius() {
        assert_eq!(
            clamp_floating_dpad_knob(CENTER, TouchPoint { x: 300.0, y: 100.0 }, 100.0),
            TouchPoint { x: 200.0, y: 100.0 }
        );
    }
}
