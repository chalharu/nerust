use std::collections::{BTreeMap, HashMap};

use nerust_core_traits::identity::SystemId;
use nerust_input_traits::{AttachmentId, DigitalControlId};
use nerust_keyboard::Key;

pub const IMPLICIT_PROFILE_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InputSettings {
    pub systems: HashMap<Box<dyn SystemId>, SystemInputSettings>,
    pub shortcuts: ShortcutSettings,
}

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
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(Key::F5),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: Some(Key::F6),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: Some(Key::F7),
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: Some(Key::F8),
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: Some(Key::F11),
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
        },
        ShortcutBinding {
            action: ShortcutAction::OpenDebugger,
            key: Some(Key::F4),
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
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShortcutBinding {
    pub action: ShortcutAction,
    pub key: Option<Key>,
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
        }];
        assert_eq!(
            shortcut_key_label(&rebound, ShortcutAction::OpenDebugger),
            Some("F2")
        );
        // Unbound actions (and unknown lookups) advertise nothing.
        let unbound = vec![ShortcutBinding {
            action: ShortcutAction::OpenDebugger,
            key: None,
        }];
        assert_eq!(
            shortcut_key_label(&unbound, ShortcutAction::OpenDebugger),
            None
        );
        assert_eq!(shortcut_key_label(&unbound, ShortcutAction::Reset), None);
    }
}
