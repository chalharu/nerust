//! Focus notifications from native backends.
//!
//! The framework only reports what happened: main-window arrivals/departures,
//! auxiliary-window focus with a view-resolved `resume` hint, and the end of
//! a native event burst. It never decides suspension: whether a departure
//! suspends content, and whether an arrival resumes it, is application policy
//! that lives view-side next to the suspension holds. Content suspension is a
//! content behavior, not a framework one; the framework pauses nothing.

/// Focus-domain notification: the inputs native adapters report.
/// Adapters translate focus callbacks and the end of an event burst.
/// `resume` is resolved view-side from the concrete aux kind before reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEvent {
    Main(bool),
    Aux { resume: bool, focused: bool },
    BurstEnd,
}
