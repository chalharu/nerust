//! Single DialogOwnership service for dialog gates.
//! Gates read the settings bridge pending id; there is no cached bool.

use super::settings;

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
}
