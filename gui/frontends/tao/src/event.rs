//! Event-loop user events: the leaf type both [`crate::menu`] (producer)
//! and [`crate::runtime`] (consumer/loop owner) depend on. [`MenuCommand`]
//! lives here too so the edge is one-directional (nothing in `event`
//! depends back on `menu` or `runtime`).

use nerust_gui_shell::session::commands::SessionCommand;

/// Native menu selections. Produced by [`crate::menu`], consumed by
/// [`crate::runtime`] via [`UserEvent::Menu`].
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum MenuCommand {
    Open,
    Close,
    Settings,
    Debugger,
    Session(SessionCommand),
    Quit,
}

/// Menu selections from the native menu and the periodic live-follow
/// pulse for the PPU viewer. User events always wake a `Wait` loop,
/// unlike timer-based waits; the tick handler no-ops unless a running
/// core has the viewer open.
#[derive(Debug)]
pub(crate) enum UserEvent {
    Menu(MenuCommand),
    LiveTick,
}
