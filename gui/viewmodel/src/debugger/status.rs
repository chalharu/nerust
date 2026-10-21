//! Debugger outcomes as data: the session reports *what*
//! happened, frontends render wording in their own language.

use nerust_core_traits::debugger::{DebuggerError, SpaceId, SpaceTable};

pub use nerust_gui_runtime::debugger::{PendingWrite, StatusMessage};

/// Resolve the display key for a [`DebuggerError`]. `None` for
/// variants without a table entry. Lives outside the kernel: error
/// display is presentation's duty, so `UnknownSpace` never
/// fabricates a key from inside domain code.
pub fn space_key(table: &SpaceTable, error: &DebuggerError) -> Option<&'static str> {
    let id: SpaceId = match error {
        DebuggerError::UnknownSpace(id) | DebuggerError::ReadOnlySpace(id) => *id,
        DebuggerError::UnmappedAddress { space, .. } => *space,
        _ => return None,
    };
    table.get(id).map(|s| s.key)
}
