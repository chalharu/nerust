//! Composition-root launch bundle. The composition root builds the session,
//! factory, and loader into an opaque [`Application`](super::app::Application)
//! through [`ApplicationBuilder`](super::app::ApplicationBuilder); native
//! frontends receive only this struct and never name shell, runtime, or
//! render types.

pub use super::app::{Application, ApplicationBuilder};
