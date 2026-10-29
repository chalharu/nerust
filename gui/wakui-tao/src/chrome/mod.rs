//! Chrome realization: the [`wakui_core::chrome`] vocabulary made native.
//! [`window`] owns the main handle and turn hookup, [`program`] adapts it to
//! iced, [`sync`] refreshes menu/title/fullscreen, [`menu`] builds the native
//! menubar, [`dialogs`] pumps modal requests, and [`aux_windows`] hosts every
//! secondary window.
pub(crate) mod aux_windows;
pub mod dialogs;
pub mod menu;
pub mod program;
pub mod sync;
pub mod window;
