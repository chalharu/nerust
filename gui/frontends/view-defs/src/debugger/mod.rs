//! Toolkit-free debugger intents, owned state, and split-lock storage.

mod bridge;
mod driver;
mod model;
mod view;

pub use bridge::{BridgeLock, BridgeView, DebugBridge, DrainSnapshot};
pub(crate) use bridge::{
    debugger_chrome, freeze_text, pending_text, ppu_chrome, selected_row_text, status_text,
    watch_text,
};
pub use driver::DebuggerDriver;
pub use model::{DebuggerModel, DebuggerReduction, DebuggerState};
pub use nerust_frontend_host::debugger::{
    DebugInputs, DebugSnapshot, DebuggerChrome, DebuggerEvent, DebuggerRows, DebuggerScope,
    DisasmRow, DumpRow, PpuChrome, RegRow, ScaledPicture, pin_marker,
};
pub use view::{AppliedDrain, AppliedLive};

use nerust_gui_shell::debug::DebugAction;

/// One ordered action batch and its native presentation effects.
/// Input echoes have neither actions nor a rebuild scope.
#[derive(Debug, Default)]
pub struct DebuggerUpdate {
    pub actions: Vec<DebugAction>,
    pub scope: Option<DebuggerScope>,
    pub open_ppu: bool,
}

#[cfg(test)]
mod tests;
