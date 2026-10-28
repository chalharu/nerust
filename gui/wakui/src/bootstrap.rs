//! Composition-root boot bundle. The composition root builds the session,
//! factory, and loader into an opaque [`Boot`](super::app::Boot); native
//! frontends receive only this struct and never name shell, runtime, or
//! render types.

pub use super::app::Boot;
