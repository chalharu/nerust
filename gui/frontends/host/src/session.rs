//! Framework session vocabulary: gestures in, outcomes out.
//! View definitions and shell execution share these boundary types; native
//! frontends construct them without naming shell or settings types.

use wakui::Key;

/// Native gesture: open the debugger, toggle fullscreen, or run a session
/// command. Frontends never name the settings binding that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutGesture {
    OpenDebugger,
    ToggleFullscreen,
    TogglePause,
    SaveActiveSlot,
    LoadActiveSlot,
    SelectNextSlot,
    SelectPreviousSlot,
    Reset,
}

/// Toolkit-independent session event. Native adapters translate callbacks
/// into these intents; policy and ordering live view-side.
#[derive(Debug, Clone, Copy)]
pub enum SessionEvent {
    Command(crate::commands::SessionCommand),
    Keyboard {
        key: Key,
        pressed: bool,
    },
    Shortcut(ShortcutGesture),
    ClearInput,
    /// Acquire before native creation; duplicate acquisition is a no-op.
    SettingsOpened,
    /// Roll back an unsuccessful native creation, just like a normal close.
    SettingsOpenFailed,
    SettingsClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEffect {
    NativeOpenDebugger,
    ToggleFullscreen,
}

/// Command execution stripped to what native adapters observe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionOutcome {
    pub command: Option<CommandOutcome>,
    pub effect: Option<SessionEffect>,
    /// Whether this event acquired/released the Settings owner. Native chrome
    /// can skip duplicate close notifications using this flag.
    pub settings_changed: bool,
}

/// Boundary error: the shell cause rendered once, never matched on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(pub String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionError {}
