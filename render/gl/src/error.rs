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

impl From<ViewError> for nerust_render_traits::renderer::RendererError {
    /// Single-error boundary: every view fault becomes one `RendererError`
    /// with a machine-readable kind plus its typed source (no string details).
    fn from(error: ViewError) -> Self {
        use nerust_render_traits::renderer::{RendererError, RendererErrorKind};
        match error {
            ViewError::MissingUniform { .. }
            | ViewError::MissingAttribute { .. }
            | ViewError::NoProgram { .. }
            | ViewError::NoVertexArray { .. }
            | ViewError::NoVertexBuffer { .. } => RendererError::with_kind(
                "program",
                RendererErrorKind::Attach {
                    backend: "gl",
                    stage: "program",
                },
                Box::new(error),
            ),
            ViewError::InvalidOsdClear { .. }
            | ViewError::InvalidOsdFlip { .. }
            | ViewError::InvalidSurface { .. } => RendererError::with_kind(
                "surface",
                RendererErrorKind::InvalidSurface,
                Box::new(error),
            ),
            ViewError::OsdUpload { stage, .. } => RendererError::with_kind(
                "upload",
                RendererErrorKind::Upload { stage },
                Box::new(error),
            ),
            ViewError::Gl(_) | ViewError::Program(_) | ViewError::Frame(_) => {
                RendererError::with_kind("view", RendererErrorKind::Other, Box::new(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ViewError;
    use nerust_render_traits::renderer::{RendererError, RendererErrorKind};

    #[test]
    fn view_errors_map_to_kinded_render_errors() {
        for (error, kind) in [
            (
                ViewError::MissingUniform {
                    name: "u",
                    program: "p",
                },
                RendererErrorKind::Attach {
                    backend: "gl",
                    stage: "program",
                },
            ),
            (
                ViewError::MissingAttribute {
                    name: "a",
                    program: "p",
                },
                RendererErrorKind::Attach {
                    backend: "gl",
                    stage: "program",
                },
            ),
            (
                ViewError::NoProgram { program: "p" },
                RendererErrorKind::Attach {
                    backend: "gl",
                    stage: "program",
                },
            ),
            (
                ViewError::NoVertexArray { pipeline: "p" },
                RendererErrorKind::Attach {
                    backend: "gl",
                    stage: "program",
                },
            ),
            (
                ViewError::NoVertexBuffer { pipeline: "p" },
                RendererErrorKind::Attach {
                    backend: "gl",
                    stage: "program",
                },
            ),
            (
                ViewError::InvalidOsdClear { w: 1, h: 2 },
                RendererErrorKind::InvalidSurface,
            ),
            (
                ViewError::InvalidOsdFlip {
                    surface_h: 1,
                    y: 2,
                    h: 3,
                },
                RendererErrorKind::InvalidSurface,
            ),
            (
                ViewError::InvalidSurface { w: 1, h: 2 },
                RendererErrorKind::InvalidSurface,
            ),
        ] {
            let text = error.to_string();
            let render: RendererError = error.into();
            assert_eq!(render.kind(), kind, "{text}");
            assert!(!render.to_string().is_empty());
        }
    }

    #[test]
    fn osd_upload_keeps_stage_and_typed_source() {
        use nerust_glwrap::{GlError, GlErrorKind};
        let gl = GlError::new(vec![GlErrorKind::InvalidValue]);
        let error = ViewError::OsdUpload {
            rect: (0, 0, 2, 1),
            stage: "blit",
            source: gl,
        };
        assert!(error.to_string().contains("blit"));
        let render: RendererError = error.into();
        assert_eq!(render.kind(), RendererErrorKind::Upload { stage: "blit" });
        // Typed source chain survives: downcast recovers the `ViewError`.
        let source = std::error::Error::source(&render).expect("source");
        assert!(source.to_string().contains("blit"));
    }
}
