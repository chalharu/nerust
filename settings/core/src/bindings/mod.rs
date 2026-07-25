pub mod descriptors;

use std::collections::BTreeMap;

use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::{input::ShortcutAction, shared::DesktopSharedSettings};
use nerust_input_traits::InputTopologyDescriptor;
use nerust_keyboard::Key;

/// Find key binding conflicts across all profiles and shortcuts.
///
/// Returns a map of conflicted keys → bound actions.
pub fn conflicting_keys(
    settings: &DesktopSharedSettings,
    _topology: &InputTopologyDescriptor,
    _system_id: &dyn SystemId,
) -> BTreeMap<Key, Vec<String>> {
    let mut keys_to_labels: BTreeMap<Key, Vec<String>> = BTreeMap::new();

    // Collect all occupied keys from keyboard bindings
    for system_input in settings.input.systems.values() {
        for profile in system_input.keyboard_profiles.values() {
            for binding in &profile.bindings {
                let label = format!(
                    "{}: {}",
                    binding.attachment.as_str(),
                    binding.control.as_str()
                );
                keys_to_labels.entry(binding.key).or_default().push(label);
            }
        }
    }

    // Collect shortcut keys
    for shortcut in &settings.input.shortcuts.keyboard {
        if let Some(key) = shortcut.key {
            let action_label = match shortcut.action {
                ShortcutAction::TogglePause => "Toggle Pause",
                ShortcutAction::SaveActiveSlot => "Save Slot",
                ShortcutAction::SelectNextSlot => "Next Slot",
                ShortcutAction::SelectPreviousSlot => "Previous Slot",
                ShortcutAction::LoadActiveSlot => "Load Slot",
                ShortcutAction::ToggleFullscreen => "Toggle Fullscreen",
                ShortcutAction::Reset => "Reset",
            };
            keys_to_labels
                .entry(key)
                .or_default()
                .push(action_label.to_string());
        }
    }

    // Filter: keep only keys with >1 action
    keys_to_labels.retain(|_, labels| labels.len() > 1);
    keys_to_labels
}
