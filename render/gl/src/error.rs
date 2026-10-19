//! GL view error vocabulary shared by the view orchestrator and its leaf modules.

/// View failures: GL errors, shader errors, invariant and frame faults.
/// Translated to a warned `RenderResult::Error` at the render boundary.
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
    #[error("GL view has no shader program")]
    NoProgram,
    #[error("GL view has no vertex array")]
    NoVertexArray,
    #[error("GL view has no vertex buffer")]
    NoVertexBuffer,
    #[error("invalid OSD clear {w}x{h}")]
    InvalidOsdClear { w: i32, h: i32 },
    #[error("invalid viewport {w}x{h}")]
    InvalidViewport { w: i32, h: i32 },
    #[error("invalid surface size")]
    InvalidSurface,
    #[error("OSD upload failed at {stage} for rect {rect:?}: {source}")]
    OsdUpload {
        rect: (i32, i32, i32, i32),
        stage: &'static str,
        #[source]
        source: nerust_glwrap::GlError,
    },
}
