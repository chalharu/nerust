//! Input map inversion: bindings → field index. Leaf module with no session imports.
//! Shared by `session` and `input` without a module cycle.

use std::collections::HashMap;

use nerust_input_traits::{AttachmentId, ControlId};
use nerust_keyboard::Key;
use nerust_settings_core::model::input::KeyboardBinding;

/// One skipped duplicate binding, reported for the caller to log.
/// First registration wins; later duplicates are dropped deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DuplicateBinding {
    pub attachment: String,
    pub control: String,
}

/// Resolve bindings at the settings boundary; first active registration wins.
/// Runs only on settings/assignment change, never per key event.
pub(crate) fn rebuild_input_map(
    field_map: &HashMap<(AttachmentId, ControlId), usize>,
    bindings: &[KeyboardBinding],
    target: &mut HashMap<Key, usize>,
) -> Result<(), DuplicateBinding> {
    nerust_settings_core::bindings::resolve_key_fields(field_map, bindings, target).map_err(
        |conflict| DuplicateBinding {
            attachment: conflict.attachment,
            control: conflict.control.as_str().to_owned(),
        },
    )
}
