//! GL view error vocabulary shared by the view orchestrator and its leaf modules.

/// View failures translated to a warned `RenderResult::Error` at the boundary.
/// `Frame` is the frame-fault path; backend faults stay fully typed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ViewError {
    #[error(transparent)]
    Gl(#[from] nerust_glwrap::GlError),
    #[error(transparent)]
    Program(#[from] nerust_glwrap::ShaderError),
    #[error(transparent)]
    Frame(#[from] crate::layout::FrameError),
    #[error("GL view misses required uniform `{name}` in `{program}` program")]
    MissingUniform {
        name: &'static str,
        program: &'static str,
    },
    #[error("GL view misses required attribute `{name}` in `{program}` program")]
    MissingAttribute {
        name: &'static str,
        program: &'static str,
    },
    #[error("GL view has no `{program}` shader program")]
    NoProgram { program: &'static str },
    #[error("GL view has no vertex array for `{pipeline}` pipeline")]
    NoVertexArray { pipeline: &'static str },
    #[error("GL view has no vertex buffer for `{pipeline}` pipeline")]
    NoVertexBuffer { pipeline: &'static str },
    #[error("invalid OSD clear {w}x{h}")]
    InvalidOsdClear { w: i32, h: i32 },
    #[error("invalid OSD row flip surface={surface_h} y={y} h={h}")]
    InvalidOsdFlip { surface_h: i32, y: i32, h: i32 },
    #[error("invalid surface size {w}x{h}")]
    InvalidSurface { w: i32, h: i32 },
    #[error("OSD upload failed at {stage} for rect {rect:?}: {source}")]
    OsdUpload {
        rect: (i32, i32, i32, i32),
        stage: &'static str,
        #[source]
        source: nerust_glwrap::GlError,
    },
}
