//! Debugger application wiring (drain driver) plus the view re-export shim.
//! View definitions live in `nerust_frontend_viewdefs::debugger`; native
//! adapters keep their `host::debugger` paths during migration.

mod driver;

pub use driver::{DebuggerDriver, DebuggerEffects, DebuggerLiveEffects};
pub use nerust_frontend_viewdefs::debugger::*;
pub use nerust_frontend_viewdefs::sync::lock_recovered;
