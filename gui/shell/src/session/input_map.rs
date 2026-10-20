//! Input map inversion: bindings → field index. Leaf module with no session imports.
//! Shared by `session` and `input` without a module cycle.

use std::collections::HashMap;

use nerust_input_traits::{AttachmentId, ControlId};
use nerust_keyboard::Key;
use nerust_settings_core::model::input::{KeyboardBinding, PersistedControlId};

/// One skipped duplicate binding, reported for the caller to log.
/// First registration wins; later duplicates are dropped deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DuplicateBinding {
    pub attachment: String,
    pub control: String,
}

/// Rebuild the keyboard map in one pass: index digital bindings, probe each field once.
/// Runs only on settings/assignment change, never per key event.
pub(crate) fn rebuild_input_map(
    field_map: &HashMap<(AttachmentId, ControlId), usize>,
    bindings: &[KeyboardBinding],
    target: &mut HashMap<Key, usize>,
) -> Result<(), DuplicateBinding> {
    target.clear();
    target.reserve(bindings.len());
    let mut by_control: HashMap<(&str, &str), &KeyboardBinding> =
        HashMap::with_capacity(bindings.len());
    let mut first_duplicate: Option<DuplicateBinding> = None;
    for binding in bindings {
        let Some(key) = binding_index_key(binding) else {
            continue;
        };
        if by_control.contains_key(&key) {
            if first_duplicate.is_none() {
                first_duplicate = Some(DuplicateBinding {
                    attachment: key.0.to_string(),
                    control: key.1.to_string(),
                });
            }
            continue;
        }
        by_control.insert(key, binding);
    }
    for ((attachment, control), &field) in field_map {
        let Some(control) = control.as_digital() else {
            continue;
        };
        if let Some(binding) = by_control.get(&(attachment.as_str(), control.as_str())) {
            target.insert(binding.key, field);
        }
    }
    match first_duplicate {
        Some(duplicate) => Err(duplicate),
        None => Ok(()),
    }
}

/// Index key for the single-pass rebuild. `None` when the binding
/// can never match a digital field (e.g. analog persisted ids).
fn binding_index_key(binding: &KeyboardBinding) -> Option<(&str, &str)> {
    match &binding.control {
        PersistedControlId::Digital(_) => {
            Some((binding.attachment.as_str(), binding.control.as_str()))
        }
        _ => None,
    }
}
