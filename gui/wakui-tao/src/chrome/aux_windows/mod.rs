//! Auxiliary windows: the [`wakui_core::chrome::aux`] realization.
//! [`registry`] routes window identities, [`event_adapt`] lifts tao events
//! into toolkit-independent input, [`shell`] hosts the iced program, and
//! [`widgets`] binds control screens to native widgets.

pub(crate) mod event_adapt;
pub(crate) mod focus;
/// Aux creation over the [`crate::host::HostState`]: reserve, create, finish.
pub(crate) mod open;
pub(crate) mod registry;
pub(crate) mod shell;
pub(crate) mod widgets;
