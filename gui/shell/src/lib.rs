//! Shell-facing adapter layer for NES console sessions.
//!
//! This crate provides the NES-specific adapter types and shell-facing runtime
//! surface that connect a generic session runtime to a concrete shell binary:
//!
//! - [`shell_api`] — re-exports the `GuiSession`, session commands, slot
//!   summaries, controller types, and window sizing that shell binaries need,
//!   so OS-specific binaries do not depend on `nerust_gui_runtime` directly.
//! - [`options`] — re-exports shell-facing console load options.
//! - [`NesConsoleDescriptor`] — builds a NES [`crate::shell_api::GuiSession`]
//!   and describes its controller layout.
//! - [`NesInputAdapter`] — translates host key/button events to NES controller
//!   inputs and flushes them to the session.
//! - [`NativeShellState`] — tracks frame-presentation and redraw timing for
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
mod descriptor;
mod input;
pub mod shell_api;
mod state;

pub use self::descriptor::NesConsoleDescriptor;
pub use self::input::NesInputAdapter;
pub use self::state::NativeShellState;
pub use nerust_gui_runtime::options;
