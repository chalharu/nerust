pub mod descriptors;

use std::collections::BTreeMap;

use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::{input::ShortcutAction, shared::DesktopSharedSettings};
use nerust_input_traits::InputTopologyDescriptor;
use nerust_keyboard::Key;

/// Find key binding conflicts within a single system.
///
/// Only the implicit keyboard profile of the given `system` is
/// checked. Shortcut conflicts across the same key are also detected.
pub fn conflicting_keys(
    settings: &DesktopSharedSettings,
    topology: &InputTopologyDescriptor,
    system: &dyn SystemId,
) -> BTreeMap<Key, Vec<String>> {
    let mut by_key = BTreeMap::<Key, Vec<String>>::new();

    if let Some(profile) = settings
        .input
        .systems
        .get(system)
        .and_then(|s| s.implicit_keyboard_profile())
    {
        for descriptor in descriptors::keyboard_binding_descriptors(topology, system) {
            if let Some(binding) = profile.bindings.iter().find(|binding| {
                binding.attachment.as_str() == descriptor.attachment.as_str()
                    && binding.control.as_str() == descriptor.control.as_str()
            }) {
                by_key.entry(binding.key).or_default().push(format!(
                    "{} {}",
                    descriptor.attachment_label, descriptor.control_label
                ));
            }
        }
    }

    for descriptor in descriptors::shortcut_descriptors() {
        if let Some(binding) = settings
            .input
            .shortcuts
            .keyboard
            .iter()
            .find(|binding| binding.action == descriptor.action)
            && let Some(key) = binding.key
        {
            by_key
                .entry(key)
                .or_default()
                .push(descriptor.label.to_string());
        }
    }

    by_key.retain(|_, labels| labels.len() > 1);
    by_key
}
