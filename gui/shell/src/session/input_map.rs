//! Input map inversion: bindings → field index. Leaf module with no session imports.
//! Shared by `session` and `input` without a module cycle.

use std::collections::HashMap;

use nerust_input_traits::UnifiedFieldMap;
use nerust_input_traits::{BUILTIN_KEYBOARD_MEMBER, Key, port_index_for_slot};
use nerust_settings_core::model::app_state::{PORT_PLAYER_1, PORT_PLAYER_2, PortDeviceSets};
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
    field_map: &UnifiedFieldMap,
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

/// Player-port indices the implicit keyboard may drive: ports whose member
/// set contains the built-in keyboard member. Desktop has a single implicit
/// keyboard, so one membership lookup covers every key event; the hot path
/// never touches port sets. Unknown port ids never match a slot-resolved
/// index, so they drop out naturally.
pub(crate) fn keyboard_port_indices(port_devices: &PortDeviceSets) -> Vec<usize> {
    [(PORT_PLAYER_1, 0), (PORT_PLAYER_2, 1)]
        .into_iter()
        .filter(|(port, _)| {
            port_devices
                .get(*port)
                .is_some_and(|members| members.iter().any(|m| m == BUILTIN_KEYBOARD_MEMBER))
        })
        .map(|(_, index)| index)
        .collect()
}

/// Whether one keyboard binding survives routing: unresolvable attachments
/// fail open (never silently drop rows for a factory addition), otherwise
/// the attachment's port must be in the keyboard's port set.
pub(crate) fn binding_reaches_port(binding: &KeyboardBinding, keyboard_ports: &[usize]) -> bool {
    port_index_for_slot(binding.attachment.as_str())
        .is_none_or(|port| keyboard_ports.contains(&port))
}
