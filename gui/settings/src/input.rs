use std::collections::BTreeMap;

use nerust_core_traits::identity::SystemId;
use nerust_input_traits::{AttachmentId, DigitalControlId};
use nerust_keyboard::Key;

pub const IMPLICIT_PROFILE_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InputSettings {
    pub systems: BTreeMap<SystemId, SystemInputSettings>,
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
