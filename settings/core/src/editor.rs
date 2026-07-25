use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::{
    input::{KeyboardBinding, PersistedAttachmentId, PersistedControlId, ShortcutAction},
    snapshot::SettingsSnapshot,
};
use nerust_keyboard::Key;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Binding {
        system: Box<dyn SystemId>,
        attachment: String,
        control: String,
    },
    Shortcut(ShortcutAction),
}

pub fn current_binding_key(snapshot: &SettingsSnapshot, target: &CaptureTarget) -> Option<Key> {
    match target {
        CaptureTarget::Binding {
            system,
            attachment,
            control,
        } => snapshot
            .shared
            .input
            .systems
            .get(system)?
            .implicit_keyboard_profile()?
            .bindings
            .iter()
            .find(|binding| {
                binding.attachment.as_str() == attachment && binding.control.as_str() == control
            })
            .map(|binding| binding.key),
        CaptureTarget::Shortcut(action) => snapshot
            .shared
            .input
            .shortcuts
            .keyboard
            .iter()
            .find(|binding| &binding.action == action)
            .and_then(|binding| binding.key),
    }
}

pub fn current_binding_label(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
) -> Option<&'static str> {
    current_binding_key(snapshot, target).map(|key| key.label())
}

pub fn apply_capture_target(
    snapshot: &mut SettingsSnapshot,
    target: &CaptureTarget,
    key: Option<Key>,
) {
    match target {
        CaptureTarget::Binding {
            system,
            attachment,
            control,
        } => {
            let profile = snapshot
                .shared
                .input
                .systems
                .entry(system.clone_box())
                .or_default()
                .implicit_keyboard_profile_mut();
            profile.bindings.retain(|binding| {
                !(binding.attachment.as_str() == attachment && binding.control.as_str() == control)
            });
            if let Some(key) = key {
                profile.bindings.push(KeyboardBinding {
                    attachment: PersistedAttachmentId::new(attachment.clone()),
                    control: PersistedControlId::digital(control.clone()),
                    key,
                });
            }
        }
        CaptureTarget::Shortcut(action) => {
            if let Some(binding) = snapshot
                .shared
                .input
                .shortcuts
                .keyboard
                .iter_mut()
                .find(|b| b.action == *action)
            {
                binding.key = key;
            }
        }
    }
}
