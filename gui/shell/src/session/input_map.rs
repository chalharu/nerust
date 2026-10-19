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

/// Generic rebuild: index bindings once, then probe per field.
/// O(F+B) instead of the O(F*B) scan; runs only on
/// settings/assignment change, never per key event.
pub(crate) fn rebuild_input_map<B: InputBinding>(
    field_map: &HashMap<(AttachmentId, DigitalControlId), usize>,
    bindings: &[B],
    target: &mut HashMap<B::Id, usize>,
) {
    target.clear();
    let mut by_control: HashMap<(&str, &str), &B> = HashMap::with_capacity(bindings.len());
    for binding in bindings {
        if let Some(key) = binding.index_key() {
            by_control.insert(key, binding);
        }
    }
    for ((attachment, control), &field) in field_map {
        if let Some(binding) = by_control.get(&(attachment.as_str(), control.as_str())) {
            target.insert(binding.source_id(), field);
        }
    }
}
