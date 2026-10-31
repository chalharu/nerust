//! Toolkit-free debugger intents, owned state, and split-lock storage.

mod bridge;
mod driver;
mod model;

pub use bridge::{BridgeLock, BridgeView, DebugBridge, DrainSnapshot, lock_recovered};
pub use driver::{DebuggerDriver, DebuggerEffects, DebuggerLiveEffects};
pub use model::{DebuggerModel, DebuggerReduction, DebuggerState};

use nerust_gui_shell::debug::DebugAction;

/// Native adapters translate gestures into these intents, never emulator calls.
#[derive(Debug, Clone)]
pub enum DebuggerEvent {
    RefreshPressed,
    SpacePrev,
    SpaceNext,
    TogglePausePressed,
    StepFramePressed,
    StepInstrPressed,
    PpuOpenPressed,
    MemInputChanged(String),
    MemGo,
    MemPage(i32),
    DisInputChanged(String),
    DisGo,
    FollowPcToggle,
    DisLineClicked(u32),
    FollowTarget(u32),
    DisBack,
    BookmarkInputChanged(String),
    BookmarkAdd,
    BookmarkJump(u32),
    BookmarkDelete(u32),
    WatchSet,
    WatchClear,
    FreezeInputChanged(String),
    FreezeSet,
    EditFreezeSet,
    FreezeClear,
    RowSelected(u32),
    EditInputChanged(String),
    EditWrite,
    ConfirmWrite,
    CancelWrite,
    RegSelected(&'static str),
    RegInputChanged(String),
    RegSet,
}

/// Minimum view sections affected by an intent (header accompanies every scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebuggerScope {
    All,
    Dump,
    Disasm,
    State,
    Edit,
    Bookmarks,
    Header,
}

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
