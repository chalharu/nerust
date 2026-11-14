use crate::model::{
    input::{
        KeyboardBinding, PadBinding, PersistedAttachmentId, PersistedControlId, ShortcutAction,
    },
    snapshot::SettingsSnapshot,
};
use nerust_core_traits::identity::SystemId;
use nerust_input_traits::{Key, StandardButton};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Binding {
        system: Box<dyn SystemId>,
        attachment: String,
        control: String,
    },
    /// One per-pad game-button override.
    PadBinding {
        system: Box<dyn SystemId>,
        pad: usize,
        attachment: String,
        control: String,
    },
    Shortcut(ShortcutAction),
}

/// One lookup for all capture columns: `(key, shortcut pad, pad override)`.
fn binding_cols(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
) -> (Option<Key>, Option<StandardButton>, Option<StandardButton>) {
    match target {
        CaptureTarget::Binding {
            system,
            attachment,
            control,
        } => {
            let key = snapshot
                .shared
                .input
                .systems
                .get(system)
                .and_then(|systems| systems.implicit_keyboard_profile())
                .and_then(|profile| {
                    profile
                        .bindings
                        .iter()
                        .find(|binding| {
                            binding.attachment.as_str() == attachment
                                && matches!(&binding.control, PersistedControlId::Digital(id) if id == control)
                        })
                })
                .map(|binding| binding.key);
            (key, None, None)
        }
        CaptureTarget::PadBinding {
            system,
            pad,
            attachment,
            control,
        } => {
            let button = snapshot
                .shared
                .input
                .systems
                .get(system)
                .and_then(|systems| systems.pad_bindings.get(*pad))
                .and_then(|bindings| {
                    bindings.iter().find(|binding| {
                        binding.attachment.as_str() == attachment
                            && matches!(&binding.control, PersistedControlId::Digital(id) if id == control)
                    })
                })
                .map(|binding| binding.button);
            (None, None, button)
        }
        CaptureTarget::Shortcut(action) => {
            let row = snapshot
                .shared
                .input
                .shortcuts
                .keyboard
                .iter()
                .find(|binding| &binding.action == action);
            (
                row.and_then(|binding| binding.key),
                row.and_then(|binding| binding.pad),
                None,
            )
        }
    }
}

pub fn current_binding_key(snapshot: &SettingsSnapshot, target: &CaptureTarget) -> Option<Key> {
    binding_cols(snapshot, target).0
}

pub fn current_binding_label(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
) -> Option<&'static str> {
    current_binding_key(snapshot, target).map(|key| key.label())
}

pub fn current_pad_binding(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
) -> Option<StandardButton> {
    binding_cols(snapshot, target).1
}

pub fn current_pad_game_binding(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
) -> Option<StandardButton> {
    binding_cols(snapshot, target).2
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
                !(binding.attachment.as_str() == attachment
                    && matches!(&binding.control, PersistedControlId::Digital(id) if id == control))
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
        // Keyboard capture never lands on a pad-mapping row.
        CaptureTarget::PadBinding { .. } => {}
    }
}

/// Apply one captured pad button; game binding rows report `false`.
pub fn apply_capture_pad_target(
    snapshot: &mut SettingsSnapshot,
    target: &CaptureTarget,
    button: Option<StandardButton>,
) -> bool {
    match target {
        CaptureTarget::Shortcut(action) => {
            if let Some(binding) = snapshot
                .shared
                .input
                .shortcuts
                .keyboard
                .iter_mut()
                .find(|b| b.action == *action)
            {
                binding.pad = button;
            }
            true
        }
        CaptureTarget::PadBinding {
            system,
            pad,
            attachment,
            control,
        } => {
            // Missing systems materialize; out-of-range pads fail.
            let settings = snapshot
                .shared
                .input
                .systems
                .entry(system.clone_box())
                .or_default();
            let Some(bindings) = settings.pad_bindings.get_mut(*pad) else {
                return false;
            };
            bindings.retain(|binding| {
                !(binding.attachment.as_str() == attachment
                    && matches!(&binding.control, PersistedControlId::Digital(id) if id == control))
            });
            if let Some(button) = button {
                bindings.push(PadBinding {
                    attachment: PersistedAttachmentId::new(attachment.clone()),
                    control: PersistedControlId::digital(control.clone()),
                    button,
                });
            }
            true
        }
        CaptureTarget::Binding { .. } => {
            log::warn!("ignoring pad capture on a game binding row");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        app_state::DesktopAppState,
        input::{
            InputSettings, KeyboardBinding, PAD_COUNT, PersistedControlId, ShortcutAction,
            ShortcutBinding, ShortcutSettings, SystemInputSettings,
        },
        local::HostBackendLocalSettings,
        shared::DesktopSharedSettings,
        snapshot::SettingsSnapshot,
    };
    use nerust_core_traits::declare_system_id;
    use nerust_input_traits::Key;

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
    fn digital_capture_never_reads_or_removes_an_analog_id_with_the_same_text() {
        let mut snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let target = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "test.ctrl".into(),
        };
        let profile = snapshot
            .shared
            .input
            .systems
            .values_mut()
            .next()
            .unwrap()
            .implicit_keyboard_profile_mut();
        profile.bindings.insert(
            0,
            KeyboardBinding::new(
                "test.att",
                PersistedControlId::analog("test.ctrl"),
                Key::KeyX,
            ),
        );
        assert_eq!(current_binding_key(&snapshot, &target), Some(Key::KeyZ));
        apply_capture_target(&mut snapshot, &target, Some(Key::KeyA));
        assert_eq!(current_binding_key(&snapshot, &target), Some(Key::KeyA));
        apply_capture_target(&mut snapshot, &target, None);
        assert_eq!(current_binding_key(&snapshot, &target), None);
        let profile = snapshot
            .shared
            .input
            .systems
            .values()
            .next()
            .unwrap()
            .implicit_keyboard_profile()
            .unwrap();
        assert_eq!(profile.bindings.len(), 1);
        assert_eq!(
            profile.bindings[0].control,
            PersistedControlId::analog("test.ctrl")
        );
        assert_eq!(profile.bindings[0].key, Key::KeyX);
    }

    #[test]
    fn pad_capture_lands_on_shortcuts_never_on_bindings() {
        use crate::model::input::default_shortcut_bindings;
        use nerust_input_traits::StandardButton;
        let mut snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        snapshot.shared.input.shortcuts.keyboard = default_shortcut_bindings();
        let shortcut = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        assert!(apply_capture_pad_target(
            &mut snapshot,
            &shortcut,
            Some(StandardButton::Start)
        ));
        assert_eq!(
            current_pad_binding(&snapshot, &shortcut),
            Some(StandardButton::Start)
        );
        assert!(apply_capture_pad_target(&mut snapshot, &shortcut, None));
        assert_eq!(current_pad_binding(&snapshot, &shortcut), None);
        let binding = CaptureTarget::Binding {
            system: Box::new(TestSysId),
            attachment: "test.att".into(),
            control: "test.ctrl".into(),
        };
        assert!(!apply_capture_pad_target(
            &mut snapshot,
            &binding,
            Some(StandardButton::Start)
        ));
        assert_eq!(current_pad_binding(&snapshot, &binding), None);
        assert_eq!(current_binding_key(&snapshot, &binding), Some(Key::KeyZ));
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
                            pad: None,
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

    fn pad_target(pad: usize) -> CaptureTarget {
        CaptureTarget::PadBinding {
            system: Box::new(TestSysId),
            pad,
            attachment: "test.att".into(),
            control: "test.ctrl".into(),
        }
    }

    #[test]
    fn pad_game_binding_round_trips_per_pad() {
        use nerust_input_traits::StandardButton;
        let mut snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let first = pad_target(0);
        let second = pad_target(1);
        assert_eq!(current_pad_game_binding(&snapshot, &first), None);
        assert!(apply_capture_pad_target(
            &mut snapshot,
            &first,
            Some(StandardButton::East)
        ));
        assert_eq!(
            current_pad_game_binding(&snapshot, &first),
            Some(StandardButton::East)
        );
        // Pads stay independent; the keyboard binding is untouched.
        assert_eq!(current_pad_game_binding(&snapshot, &second), None);
        assert_eq!(
            current_binding_key(
                &snapshot,
                &CaptureTarget::Binding {
                    system: Box::new(TestSysId),
                    attachment: "test.att".into(),
                    control: "test.ctrl".into(),
                }
            ),
            Some(Key::KeyZ)
        );
        // Re-capture replaces; clear removes the override.
        assert!(apply_capture_pad_target(
            &mut snapshot,
            &first,
            Some(StandardButton::West)
        ));
        assert_eq!(
            current_pad_game_binding(&snapshot, &first),
            Some(StandardButton::West)
        );
        assert!(apply_capture_pad_target(&mut snapshot, &first, None));
        assert_eq!(current_pad_game_binding(&snapshot, &first), None);
    }

    #[test]
    fn pad_game_binding_rejects_out_of_range_pads() {
        use nerust_input_traits::StandardButton;
        let mut snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        assert!(!apply_capture_pad_target(
            &mut snapshot,
            &pad_target(PAD_COUNT),
            Some(StandardButton::South)
        ));
    }

    #[test]
    fn pad_game_binding_ignores_keyboard_capture() {
        let mut snapshot = snapshot_with_binding("test.att", "test.ctrl", Key::KeyZ);
        let target = pad_target(0);
        apply_capture_target(&mut snapshot, &target, Some(Key::KeyA));
        assert_eq!(current_pad_game_binding(&snapshot, &target), None);
        assert_eq!(current_binding_key(&snapshot, &target), None);
    }
}
