use super::Vec2D;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub(crate) struct VertexData {
    pub(crate) position: Vec2D,
    pub(crate) uv: Vec2D,
}

impl VertexData {
    pub(crate) fn new(position: Vec2D, uv: Vec2D) -> Self {
        Self { position, uv }
    }
}
