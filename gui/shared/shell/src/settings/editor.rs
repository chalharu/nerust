use crate::settings::bindings::keys::keyboard_key_label;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::input::{
    KeyboardBinding, KeyboardKey, PersistedAttachmentId, PersistedControlId, ShortcutAction,
};
use nerust_input_schema::SystemId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Binding {
        system: SystemId,
        attachment: String,
        control: String,
    },
    Shortcut(ShortcutAction),
}

pub fn current_binding_key(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
) -> Option<KeyboardKey> {
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
    current_binding_key(snapshot, target).map(keyboard_key_label)
}

pub fn apply_capture_target(
    snapshot: &mut SettingsSnapshot,
    target: &CaptureTarget,
    key: Option<KeyboardKey>,
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
                .entry(*system)
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
                .find(|binding| binding.action == *action)
            {
                binding.key = key;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureTarget, apply_capture_target, current_binding_key};
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
    use nerust_input_nes::topology::{NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A};
    use nerust_input_schema::SystemId;

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        }
    }

    #[test]
    fn reads_existing_control_binding() {
        let snapshot = snapshot();

        assert_eq!(
            current_binding_key(
                &snapshot,
                &CaptureTarget::Binding {
                    system: SystemId::Nes,
                    attachment: NES_ATTACHMENT_PLAYER_ONE.as_str().into(),
                    control: NES_CONTROL_A.as_str().into(),
                }
            ),
            Some(KeyboardKey::KeyZ)
        );
    }

    #[test]
    fn updates_existing_control_binding() {
        let mut snapshot = snapshot();
        let target = CaptureTarget::Binding {
            system: SystemId::Nes,
            attachment: NES_ATTACHMENT_PLAYER_ONE.as_str().into(),
            control: NES_CONTROL_A.as_str().into(),
        };

        apply_capture_target(&mut snapshot, &target, Some(KeyboardKey::KeyA));

        assert_eq!(
            current_binding_key(&snapshot, &target),
            Some(KeyboardKey::KeyA)
        );
    }

    #[test]
    fn clears_existing_shortcut_binding() {
        let mut snapshot = snapshot();
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);

        apply_capture_target(&mut snapshot, &target, None);

        assert_eq!(current_binding_key(&snapshot, &target), None);
    }
}
