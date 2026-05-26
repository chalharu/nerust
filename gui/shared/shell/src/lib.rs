//! System-neutral shell/session orchestration.
//!
//! This crate owns the shared orchestration layer used by native frontends:
//!
//! - [`session::SessionHandle`] — host-facing session orchestrator.
//! - [`session::SessionSnapshot`] — owned read model for host/runtime consumers.
//! - [`descriptor`] — system definition, input adapter, runtime, and settings page contracts.
//! - [`load`] — system-neutral media/load request types.
//!
//! `NativeShellState` lives in `nerust_gui_runtime::shell` because it is common
//! to native hosts regardless of system.
pub mod descriptor;
pub mod load;
pub mod session;
pub mod settings;
