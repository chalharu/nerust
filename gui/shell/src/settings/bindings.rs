pub mod descriptors;
pub mod events;
pub mod keys;

use std::collections::BTreeMap;

use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::shared::DesktopSharedSettings;
use nerust_input_traits::InputTopologyDescriptor;
use nerust_keyboard::Key;

pub fn conflicting_keys(
    settings: &DesktopSharedSettings,
    topology: &InputTopologyDescriptor,
    system: SystemId,
) -> BTreeMap<Key, Vec<String>> {
    let mut by_key = BTreeMap::<Key, Vec<String>>::new();

    if let Some(profile) = settings
        .input
        .systems
        .get(&system)
        .and_then(|system| system.implicit_keyboard_profile())
    {
        for descriptor in descriptors::keyboard_binding_descriptors(topology) {
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

#[cfg(test)]
mod tests {
    use nerust_core_traits::identity::SystemId;
    use nerust_keyboard::Key;

    use super::conflicting_keys;
    use crate::{
        settings::defaults::seed::default_shared_settings, test_support::single_port_topology,
    };

    #[test]
    fn detects_conflicts_across_controls_and_shortcuts() {
        let mut settings = default_shared_settings(&[]);
        settings
            .input
            .shortcuts
            .keyboard
            .iter_mut()
            .find(|binding| {
                matches!(
                    binding.action,
                    nerust_gui_settings::input::ShortcutAction::TogglePause
                )
            })
            .unwrap()
            .key = Some(Key::KeyZ);

        let conflicts = conflicting_keys(&settings, &single_port_topology(), SystemId::new("nes"));
        assert!(conflicts.contains_key(&Key::KeyZ));
    }
}
