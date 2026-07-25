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

#[cfg(test)]
mod tests {
    use nerust_core_traits::declare_system_id;
    use nerust_gui_settings::{
        app_state::DesktopAppState,
        input::{
            InputSettings, KeyboardBinding, PersistedControlId, ShortcutAction, ShortcutBinding,
            ShortcutSettings, SystemInputSettings,
        },
        local::HostBackendLocalSettings,
        shared::DesktopSharedSettings,
        snapshot::SettingsSnapshot,
    };
    use nerust_keyboard::Key;

    use super::*;

    declare_system_id!(pub TestSysId, "test");

    fn snapshot_with_binding(attachment: &str, control: &str, key: Key) -> SettingsSnapshot {
        let mut snapshot = SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            attachment.to_string(),
            PersistedControlId::digital(control.to_string()),
            key,
        ));
        snapshot
            .shared
            .input
            .systems
            .insert(Box::new(TestSysId), sys);
        snapshot
    }

    #[test]
    fn current_binding_key_finds_existing() {
        let snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let target = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "test.ctrl".into(),
        };
        assert_eq!(current_binding_key(&snapshot, &target), Some(Key::KeyZ));
    }

    #[test]
    fn current_binding_key_returns_none_for_missing() {
        let snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let target = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "other.ctrl".into(),
        };
        assert_eq!(current_binding_key(&snapshot, &target), None);
    }

    #[test]
    fn apply_capture_target_adds_binding() {
        let mut snapshot = snapshot_with_binding("test.att", "existing.ctrl", Key::KeyX);
        let target = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "new.ctrl".into(),
        };
        apply_capture_target(&mut snapshot, &target, Some(Key::KeyA));
        let result = current_binding_key(&snapshot, &target);
        assert_eq!(result, Some(Key::KeyA));
    }

    #[test]
    fn apply_capture_target_clears_binding() {
        let mut snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let target = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "test.ctrl".into(),
        };
        apply_capture_target(&mut snapshot, &target, None);
        let result = current_binding_key(&snapshot, &target);
        assert_eq!(result, None);
    }

    #[test]
    fn apply_capture_target_shortcut() {
        let mut snapshot = SettingsSnapshot {
            shared: DesktopSharedSettings {
                input: InputSettings {
                    shortcuts: ShortcutSettings {
                        keyboard: vec![ShortcutBinding {
                            action: ShortcutAction::TogglePause,
                            key: None,
                        }],
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        apply_capture_target(&mut snapshot, &target, Some(Key::Space));
        let result = current_binding_key(&snapshot, &target);
        assert_eq!(result, Some(Key::Space));
    }

    #[test]
    fn current_binding_label_returns_key_name() {
        let snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let target = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "test.ctrl".into(),
        };
        let label = current_binding_label(&snapshot, &target);
        assert_eq!(label, Some("Z"));
    }
}
