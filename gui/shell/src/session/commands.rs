#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    Pause,
    Resume,
    TogglePause,
    Reset,
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectActiveSlot(u64),
    SaveSlot(u64),
    LoadSlot(u64),
    DeleteSlot(u64),
    SelectNextSlot,
    SelectPreviousSlot,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
    /// Why a slot save/load did not execute (`None` on success and for
    /// commands that cannot fail this way). Frontends map this to a
    /// user-facing message; the full detail stays in the logs.
    pub slot_failure: Option<SlotOpFailure>,
}

/// Machine-readable reason a state-slot save/load failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOpFailure {
    /// No active slot (nothing saved yet, or no core loaded).
    Empty,
    /// The slot file does not exist.
    Missing,
    /// Storage/backend I/O failure (filesystem, Android SAF, ...).
    Storage,
    /// The slot archive or its payload does not decode.
    Corrupt,
    /// The core rejected the payload (foreign ROM, options or schema
    /// mismatch, invalid state).
    Incompatible,
    /// Cannot operate (no states dir, no media identity, core export
    /// or thread failure).
    Unavailable,
}
