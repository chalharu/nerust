//! Single DialogOwnership service for dialog gates.
//!
//! Previously two sources disagreed: a cached `settings_open` bool on the
//! frontend (updated on events) and the settings bridge's pending-request
//! truth. The sync JNI show path left the cache stale (dialog up while the
//! cache read `false`), so the error dialog could evict just-opened
//! settings. Every gate now queries this service, which reads the bridge
//! pending id directly. There is no cached bool anymore.

use super::settings;

/// Sole owner of "is a dialog up" answers. Stateless by design: the
/// settings bridge pending-request id is the truth, so a query can never
/// go stale between the event tick and the gate.
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
