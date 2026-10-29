//! Chrome realization: the [`wakui_core::chrome`] vocabulary made native.
//! [`window`] owns the main handle and actions, [`menu`] builds the native
//! menubar, [`dialogs`] presents queued modals, [`aux_windows`] hosts every
//! secondary window, and [`surface`] presents the viewport plus wakeups.
pub(crate) mod aux_windows;
pub(crate) mod dialogs;
pub(crate) mod menu;
pub(crate) mod surface;
pub(crate) mod surface_state;
pub(crate) mod window;
