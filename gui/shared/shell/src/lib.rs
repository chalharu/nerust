//! NES-specific shell/session composition.
//!
//! This crate owns the current NES system-facing shell adapters:
//!
//! - [`descriptor::NesConsoleProfile`] — describes NES topology and builds the
//!   default NES console/session composition.
//! - [`load`] — shell-facing NES load options kept separate from
//!   `nerust_contract_options`.
//! - [`session::NesSession`] — wraps the generic [`nerust_gui_runtime::session::GuiSession`]
//!   with NES controller/input behavior.
//!
//! `NativeShellState` lives in `nerust_gui_runtime::shell` because it is common
//! to native hosts regardless of system.
pub mod descriptor;
pub mod load;
pub mod session;
pub mod settings;
