//! `wakui_gtk`: wakui platform backend for gtk4 (MAUI handler role).
//!
//! Framework-only: depends on [`wakui`] plus the host vocabulary
//! (`wakui` vocabulary) and gtk4. Never on view definitions,
//! sessions, or settings. Native frontends retain [`controls`] dialogs
//! from [`wakui::control::ControlScreen`] snapshots, interpret them
//! through [`renderer`], and map native input through [`keyboard`].
//!
//! Dialog lifecycle helpers live in [`dialog`]. Modal presentation stays
//! native; this crate only finishes retained dialogs on close signals.

pub mod backend;
pub mod controls;
pub mod dialog;
pub mod keyboard;

pub use backend::{GtkBackend, register};
