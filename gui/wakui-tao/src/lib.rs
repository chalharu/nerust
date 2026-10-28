//! `wakui_tao`: wakui platform backend for iced/tao (MAUI handler role).
//!
//! Framework-only: depends on [`wakui`] plus the host vocabulary
//! (`wakui` vocabulary) and the iced/tao toolkits. Never on view
//! definitions, sessions, or settings. Native frontends render
//! [`wakui::control::ControlScreen`] snapshots through [`controls`],
//! adapt them to host windows through [`binding`], and map native input
//! through [`keyboard`], [`tao_map`], and [`convert`].
//!
//! A third-party app builds UI with `wakui` plus this crate only:
//! see `examples/backend_smoke.rs`. Window event loops stay native;
//! this crate owns control interpretation, not the loop itself.

pub mod backend;
pub mod binding;
pub mod controls;
pub mod convert;
pub mod disabled;
pub mod domain;
pub mod handler;
pub mod keyboard;
pub mod tao_map;
pub mod window;

pub use backend::{TaoBackend, register};
