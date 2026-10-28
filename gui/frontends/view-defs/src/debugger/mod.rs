//! Toolkit-free debugger intents, owned state, and split-lock storage.

mod bridge;
mod driver;
mod model;
mod screen;
mod view;
pub mod vocabulary;

pub use bridge::{BridgeLock, BridgeView, DebugBridge, DrainSnapshot};
pub(crate) use bridge::{
    debugger_chrome, freeze_text, pending_text, selected_row_text, status_text, watch_text,
};
pub use driver::DebuggerDriver;
pub use model::{DebuggerModel, DebuggerReduction, DebuggerState};
pub use screen::DebuggerScreenSession;
pub use view::{AppliedDrain, AppliedLive};
pub use vocabulary::{
    DebugHandle, DebugInputs, DebugSnapshot, DebuggerChrome, DebuggerEffects, DebuggerEvent,
    DebuggerLiveEffects, DebuggerRows, DebuggerScope, DisasmRow, DumpRow, RegRow, pin_marker,
};

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
