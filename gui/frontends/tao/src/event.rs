//! Event-loop user events: the leaf type both [`crate::menu`] (producer)
//! and [`crate::runtime`] (consumer/loop owner) depend on.

use crate::menu::MenuCommand;

/// Menu selections from the native menu and the periodic live-follow
/// pulse for the PPU viewer. User events always wake a `Wait` loop,
/// unlike timer-based waits; the tick handler no-ops unless a running
/// core has the viewer open.
#[derive(Debug)]
pub(crate) enum UserEvent {
    Menu(MenuCommand),
    LiveTick,
}
