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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchOverlayAction {
    Input(DigitalInputEvent),
}
