use std::collections::{BTreeMap, HashMap};

use nerust_core_traits::identity::SystemId;
use nerust_input_traits::Key;
use nerust_input_traits::{AttachmentId, DigitalControlId, StandardButton};

pub const IMPLICIT_PROFILE_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InputSettings {
    pub systems: HashMap<Box<dyn SystemId>, SystemInputSettings>,
    pub shortcuts: ShortcutSettings,
    pub gamepad: GamepadSettings,
}

/// Pad-level preferences shared by every system: one deadzone for all
/// sticks plus the connection-order port table. Pad `i` (first-seen order
/// within a boot, stabilized by device descriptors) drives its assigned
/// port; `slot: None` parks the pad. `triggers_as_shoulders` doubles the
/// analog triggers as the shoulder buttons of the same pad (full-press
/// past 0.9): opt-in per pad, never a silent global alias.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GamepadSettings {
    /// Stick deadzone in percent, `0..=90`. Session clamps on read.
    pub deadzone_percent: u8,
    pub pad_ports: [PadPort; nerust_input_traits::MAX_PADS],
}

/// One connection-order pad: its player slot plus the trigger doubling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PadPort {
    pub slot: Option<usize>,
    pub triggers_as_shoulders: bool,
}

impl Default for GamepadSettings {
    fn default() -> Self {
        Self {
            deadzone_percent: 15,
            pad_ports: [
                PadPort {
                    slot: Some(0),
                    triggers_as_shoulders: false,
                },
                PadPort {
                    slot: Some(1),
                    triggers_as_shoulders: false,
                },
                PadPort::default(),
                PadPort::default(),
            ],
        }
    }
}

/// Maximum pads the port table (and the session order) tracks.

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SystemInputSettings {
    pub keyboard_profiles: BTreeMap<String, KeyboardProfile>,
}

impl SystemInputSettings {
    pub fn implicit_keyboard_profile(&self) -> Option<&KeyboardProfile> {
        self.keyboard_profiles.get(IMPLICIT_PROFILE_ID)
    }

    pub fn implicit_keyboard_profile_mut(&mut self) -> &mut KeyboardProfile {
        self.keyboard_profiles
            .entry(IMPLICIT_PROFILE_ID.to_string())
            .or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct KeyboardProfile {
    pub bindings: Vec<KeyboardBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeyboardBinding {
    pub attachment: PersistedAttachmentId,
    pub control: PersistedControlId,
    pub key: Key,
}

impl KeyboardBinding {
    pub fn new(attachment: impl Into<String>, control: PersistedControlId, key: Key) -> Self {
        Self {
            attachment: PersistedAttachmentId::new(attachment),
            control,
            key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShortcutSettings {
    pub keyboard: Vec<ShortcutBinding>,
}

/// Canonical default shortcut bindings (single source: the seeder and
/// the load-time migration both use this).
pub fn default_shortcut_bindings() -> Vec<ShortcutBinding> {
    vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Key::Space),
            // The out-of-box pad button: Guide pauses, everything else
            // unbound until the user captures it.
            pad: Some(StandardButton::Guide),
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(Key::F5),
            pad: None,
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: Some(Key::F6),
            pad: None,
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: Some(Key::F7),
            pad: None,
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: Some(Key::F8),
            pad: None,
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: Some(Key::F11),
            pad: None,
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
            pad: None,
        },
        ShortcutBinding {
            action: ShortcutAction::OpenDebugger,
            key: Some(Key::F4),
            pad: None,
        },
    ]
}

/// Display label for an action's current binding (`F4`, `Space`),
/// or `None` when the action is unbound. Menu builders use this so
/// rebinding (or unbinding) in Settings updates the advertised
/// shortcut instead of showing a stale hardcoded default.
pub fn shortcut_key_label(
    bindings: &[ShortcutBinding],
    action: ShortcutAction,
) -> Option<&'static str> {
    bindings
        .iter()
        .find(|binding| binding.action == action)
        .and_then(|binding| binding.key.as_ref())
        .map(|key| key.label())
}

/// Pad shortcut conflicts: one button driving two or more actions.
/// Keyboard conflicts ride `conflicting_keys`; pads are global (no system
/// topology), so they get their own map. First binding still wins at
/// runtime; this only feeds validation.
pub fn pad_shortcut_conflicts(
    bindings: &[ShortcutBinding],
) -> std::collections::BTreeMap<StandardButton, Vec<ShortcutAction>> {
    let mut by_button = std::collections::BTreeMap::<StandardButton, Vec<ShortcutAction>>::new();
    for binding in bindings {
        if let Some(button) = binding.pad {
            by_button.entry(button).or_default().push(binding.action);
        }
    }
    by_button.retain(|_, actions| actions.len() > 1);
    by_button
}

/// Migrate persisted shortcuts forward: append defaults for actions
/// the file predates (e.g. `OpenDebugger` on files seeded before it
/// existed). User bindings are never touched; when the default key is
/// already taken by another action the migrated entry stays unbound
/// (visible and rebindable in Settings) instead of stealing the key.
pub fn ensure_default_shortcuts(settings: &mut ShortcutSettings) {
    for default in default_shortcut_bindings() {
        if settings
            .keyboard
            .iter()
            .any(|binding| binding.action == default.action)
        {
            continue;
        }
        let key = match default.key {
            Some(key)
                if settings
                    .keyboard
                    .iter()
                    .any(|binding| binding.key == Some(key)) =>
            {
                None
            }
            key => key,
        };
        settings.keyboard.push(ShortcutBinding {
            action: default.action,
            key,
            pad: default.pad,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShortcutBinding {
    pub action: ShortcutAction,
    pub key: Option<Key>,
    #[serde(default)]
    pub pad: Option<StandardButton>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    TogglePause,
    SaveActiveSlot,
    SelectNextSlot,
    SelectPreviousSlot,
    LoadActiveSlot,
    ToggleFullscreen,
    Reset,
    /// Open the debugger window (default F4). Schema-driven like
    /// every other shortcut: users rebind or unbind it in Settings.
    OpenDebugger,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct PersistedAttachmentId(String);

impl PersistedAttachmentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq<AttachmentId> for PersistedAttachmentId {
    fn eq(&self, other: &AttachmentId) -> bool {
        self.0 == other.as_str()
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum PersistedControlId {
    Digital(String),
    Analog(String),
}

impl PersistedControlId {
    pub fn digital(value: impl Into<String>) -> Self {
        Self::Digital(value.into())
    }

    pub fn analog(value: impl Into<String>) -> Self {
        Self::Analog(value.into())
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Digital(value) | Self::Analog(value) => value.as_str(),
        }
    }
}

impl PartialEq<DigitalControlId> for PersistedControlId {
    fn eq(&self, other: &DigitalControlId) -> bool {
        matches!(self, Self::Digital(v) if v == other.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_shortcut_conflicts_groups_shared_buttons() {
        use super::pad_shortcut_conflicts;
        let defaults = default_shortcut_bindings();
        assert!(pad_shortcut_conflicts(&defaults).is_empty());
        let mut doubled = defaults;
        doubled
            .iter_mut()
            .find(|binding| binding.action == ShortcutAction::Reset)
            .expect("reset row")
            .pad = Some(StandardButton::Guide);
        let conflicts = pad_shortcut_conflicts(&doubled);
        assert_eq!(conflicts.len(), 1);
        let actions = &conflicts[&StandardButton::Guide];
        assert!(actions.contains(&ShortcutAction::TogglePause));
        assert!(actions.contains(&ShortcutAction::Reset));
    }

    #[test]
    fn gamepad_defaults_park_extra_pads_without_alias() {
        let gamepad = super::GamepadSettings::default();
        assert_eq!(gamepad.deadzone_percent, 15);
        assert_eq!(gamepad.pad_ports[0].slot, Some(0));
        assert_eq!(gamepad.pad_ports[1].slot, Some(1));
        assert_eq!(gamepad.pad_ports[2].slot, None);
        assert!(
            gamepad
                .pad_ports
                .iter()
                .all(|port| !port.triggers_as_shoulders)
        );
    }

    #[test]
    fn ensure_adds_missing_actions_without_touching_users() {
        let mut settings = ShortcutSettings {
            keyboard: Vec::new(),
        };
        ensure_default_shortcuts(&mut settings);
        assert_eq!(settings.keyboard, default_shortcut_bindings());
        // Second run is a no-op: nothing duplicates.
        ensure_default_shortcuts(&mut settings);
        assert_eq!(settings.keyboard, default_shortcut_bindings());
    }

    #[test]
    fn ensure_leaves_taken_keys_unbound() {
        // A legacy file that rebound F4 to pause: the migrated
        // debugger entry must not steal it back.
        let mut settings = ShortcutSettings {
            keyboard: vec![ShortcutBinding {
                action: ShortcutAction::TogglePause,
                key: Some(Key::F4),
                pad: None,
            }],
        };
        ensure_default_shortcuts(&mut settings);
        let debugger = settings
            .keyboard
            .iter()
            .find(|binding| binding.action == ShortcutAction::OpenDebugger)
            .expect("migrated entry");
        assert_eq!(debugger.key, None);
        let pause = settings
            .keyboard
            .iter()
            .find(|binding| binding.action == ShortcutAction::TogglePause)
            .expect("user binding");
        assert_eq!(pause.key, Some(Key::F4));
    }

    #[test]
    fn shortcut_key_label_follows_rebinding() {
        // Defaults advertise F4/F5.
        let defaults = default_shortcut_bindings();
        assert_eq!(
            shortcut_key_label(&defaults, ShortcutAction::OpenDebugger),
            Some("F4")
        );
        // A rebound action advertises the new key, never the default.
        let rebound = vec![ShortcutBinding {
            action: ShortcutAction::OpenDebugger,
            key: Some(Key::F2),
            pad: None,
        }];
        assert_eq!(
            shortcut_key_label(&rebound, ShortcutAction::OpenDebugger),
            Some("F2")
        );
        // Unbound actions (and unknown lookups) advertise nothing.
        let unbound = vec![ShortcutBinding {
            action: ShortcutAction::OpenDebugger,
            key: None,
            pad: None,
        }];
        assert_eq!(
            shortcut_key_label(&unbound, ShortcutAction::OpenDebugger),
            None
        );
        assert_eq!(shortcut_key_label(&unbound, ShortcutAction::Reset), None);
    }
}
