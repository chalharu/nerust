//! Viewport geometry vocabulary: logical pixels, physical pixels, surface extents.
//! Framework-owned; the render engine imports these from here.

#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub struct LogicalSize {
    pub width: usize,
    pub height: usize,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone)]
pub struct PhysicalSize {
    pub width: f32,
    pub height: f32,
}

impl From<LogicalSize> for PhysicalSize {
    fn from(value: LogicalSize) -> PhysicalSize {
        PhysicalSize {
            width: value.width as f32,
            height: value.height as f32,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

impl SurfaceSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}
