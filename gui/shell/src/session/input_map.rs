//! Input map inversion: bindings → field index. Leaf module with no
//! session imports, so both `session` (cluster owner) and `input`
//! (event path) can use it without a module cycle.

use std::{collections::HashMap, hash::Hash};

use nerust_input_traits::{AttachmentId, DigitalControlId};
use nerust_keyboard::Key;
use nerust_settings_core::model::input::KeyboardBinding;

/// Abstraction over a binding type (keyboard, gamepad, etc.) for building
/// a source-key → field-index map from an InputAssignments field_map.
pub(crate) trait InputBinding {
    type Id: Copy + Eq + Hash;
    fn source_id(&self) -> Self::Id;
    /// Index key for the O(F+B) rebuild. `None` when the binding
    /// can never match a digital field (e.g. analog persisted ids).
    fn index_key(&self) -> Option<(&str, &str)>;
}

impl InputBinding for KeyboardBinding {
    type Id = Key;
    fn source_id(&self) -> Self::Id {
        self.key
    }
    fn index_key(&self) -> Option<(&str, &str)> {
        match &self.control {
            nerust_settings_core::model::input::PersistedControlId::Digital(_) => {
                Some((self.attachment.as_str(), self.control.as_str()))
            }
            _ => None,
        }
    }
}

/// One skipped duplicate binding. The leaf reports; the caller logs.
/// First registration wins, so the map never depends on hash
/// iteration order: fields probe in sorted attachment/control order
/// and later duplicates are dropped deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DuplicateBinding {
    pub attachment: String,
    pub control: String,
}

/// Generic rebuild: index bindings once, then probe per field.
/// O(F+B) instead of the O(F*B) scan; runs only on
/// settings/assignment change, never per key event.
///
/// Deterministic: fields probe in sorted attachment/control order,
/// and duplicate bindings keep the first registration (later ones
/// are reported as [`DuplicateBinding`] for the caller to log).
pub(crate) fn rebuild_input_map<B: InputBinding>(
    field_map: &HashMap<(AttachmentId, DigitalControlId), usize>,
    bindings: &[B],
    target: &mut HashMap<B::Id, usize>,
) -> Result<(), DuplicateBinding> {
    target.clear();
    target.reserve(bindings.len());
    let mut by_control: HashMap<(&str, &str), &B> = HashMap::with_capacity(bindings.len());
    let mut first_duplicate: Option<DuplicateBinding> = None;
    for binding in bindings {
        if let Some(key) = binding.index_key() {
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
    }
    let mut fields: Vec<(&(AttachmentId, DigitalControlId), &usize)> =
        Vec::with_capacity(field_map.len());
    fields.extend(field_map.iter());
    fields.sort_by(|a, b| (a.0.0.as_str(), a.0.1.as_str()).cmp(&(b.0.0.as_str(), b.0.1.as_str())));
    for ((attachment, control), &field) in fields {
        if let Some(binding) = by_control.get(&(attachment.as_str(), control.as_str())) {
            target.insert(binding.source_id(), field);
        }
    }
    match first_duplicate {
        Some(duplicate) => Err(duplicate),
        None => Ok(()),
    }
}
