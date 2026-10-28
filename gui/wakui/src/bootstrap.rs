//! Composition-root boot bundle. The composition root builds the session,
//! factory, and loader into an opaque [`Boot`]; native frontends receive
//! only this struct and never name shell, runtime, or render types.

use super::app::FrontendApp;

/// Built session bundle handed from the composition root to a frontend.
pub struct Boot {
    pub app: Box<dyn FrontendApp>,
}
