//! Single DialogOwnership service for dialog gates.
//! Gates read the settings bridge pending id; there is no cached bool.

use super::settings;
use crate::error_dialog_controller::Screen;

/// Sole owner of "is a dialog up" answers. Reads the bridge
/// pending-request id, so a query can never go stale.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct DialogOwnership;

impl DialogOwnership {
    pub(super) fn new() -> Self {
        Self
    }

    /// Whether the settings dialog is currently up (either show path).
    pub(super) fn is_settings_open(&self) -> bool {
        settings::is_dialog_open()
    }

    /// Single gate construction for both lanes: every dialog decision
    /// routes through `Screen::show_gate`, never a local bool.
    pub(super) fn screen(&self, is_resumed: bool, can_pause: bool) -> Screen {
        Screen::show_gate(is_resumed, self.is_settings_open(), can_pause)
    }
}
