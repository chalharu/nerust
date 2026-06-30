#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub(crate) struct Vec2D {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Vec2D {
    pub(crate) fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
