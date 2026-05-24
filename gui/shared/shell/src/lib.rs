//! Shell-facing adapter layer for NES console sessions.
//!
//! This crate provides the NES-specific adapter types and shell-facing runtime
//! surface that connect a generic session runtime to a concrete shell binary:
//!
//! - [`descriptor::NesConsoleDescriptor`] — builds a NES
//!   [`nerust_gui_runtime::session::GuiSession`] and describes its controller layout.
//! - [`input::NesInputAdapter`] — translates host key/button events to NES controller
//!   inputs and flushes them to the session.
//! - [`state::NativeShellState`] — tracks frame-presentation and redraw timing for
//!   native window shells.
//!
//! # Shell × Backend Composition Policy
//!
//! Backend selection is fixed at **build-time / binary level**. Each shell
//! binary links against exactly one backend crate (`nerust_backend_opengl` or
//! `nerust_backend_wgpu`) and there is **no runtime mechanism for switching
//! backends** while the application is running.
//!
//! To add a new rendering backend, create a new binary target crate that
//! composes `nerust_gui_shell` with the new backend. Do not add runtime
//! dispatch or feature-flag backend selection to this crate.
//!
//! Current shipped combinations:
//! - `nerust_gtk`    → `nerust_backend_opengl` (GTK 3 + OpenGL 3.3)
//! - `nerust_glutin` → `nerust_backend_opengl` (winit + glutin + OpenGL 3.3)
//! - `nerust_wgpu`   → `nerust_backend_wgpu`   (tao + wgpu)
pub mod descriptor;
pub mod input;
pub mod state;
