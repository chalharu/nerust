use std::{collections::BTreeMap, path::PathBuf};

pub mod language {
    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Default,
        serde::Serialize,
        serde::Deserialize,
        strum::EnumIter,
        strum::Display,
    )]
    #[serde(rename_all = "snake_case")]
    #[strum(serialize_all = "kebab_case")]
    pub enum AppLanguage {
        #[default]
        #[strum(serialize = "System Default")]
        SystemDefault,
        Japanese,
        English,
    }
}

pub mod input {
    use nerust_core_traits::identity::SystemId;
    use nerust_input_traits::{AttachmentId, DigitalControlId};
    use nerust_keyboard::Key;

    use super::BTreeMap;

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
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Hash,
        serde::Serialize,
        serde::Deserialize,
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
}

pub mod nes {
    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Default,
        serde::Serialize,
        serde::Deserialize,
        strum::EnumIter,
        strum::Display,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum Mmc3IrqVariant {
        #[default]
        Sharp,
        Nec,
    }

    #[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct NesSettings {
        pub video: NesVideoSettings,
        pub core: NesCoreSettings,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct NesVideoSettings {
        pub filter: NesVideoFilter,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct NesCoreSettings {
        pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
    }

    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Default,
        serde::Serialize,
        serde::Deserialize,
        strum::EnumIter,
        strum::Display,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum NesVideoFilter {
        None,
        #[default]
        #[strum(serialize = "NTSC Composite")]
        NtscComposite,
        #[strum(serialize = "NTSC S-Video")]
        NtscSVideo,
        #[strum(serialize = "NTSC RGB")]
        NtscRgb,
    }
}

pub mod shared {
    use nerust_core_traits::identity::SystemId;

    use super::{BTreeMap, PathBuf, input::InputSettings, language::AppLanguage, nes::NesSettings};

    pub const DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION: u32 = 1;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct DesktopSharedSettings {
        pub schema_version: u32,
        pub general: GeneralSettings,
        pub persistence: PersistenceSettings,
        pub input: InputSettings,
        pub systems: BTreeMap<SystemId, SystemSettings>,
    }

    impl Default for DesktopSharedSettings {
        fn default() -> Self {
            Self {
                schema_version: DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION,
                general: GeneralSettings::default(),
                persistence: PersistenceSettings::default(),
                input: InputSettings::default(),
                systems: BTreeMap::new(),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct GeneralSettings {
        pub language: AppLanguage,
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct PersistenceSettings {
        pub storage_policy: StoragePolicy,
        pub storage_directory: Option<PathBuf>,
    }

    impl Default for PersistenceSettings {
        fn default() -> Self {
            Self {
                storage_policy: StoragePolicy::Sidecar,
                storage_directory: None,
            }
        }
    }

    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Default,
        serde::Serialize,
        serde::Deserialize,
        strum::EnumIter,
        strum::Display,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum StoragePolicy {
        #[default]
        Sidecar,
        AppSharedData,
        CustomDirectory,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(tag = "system", content = "settings", rename_all = "snake_case")]
    pub enum SystemSettings {
        Nes(NesSettings),
    }

    impl SystemSettings {
        pub fn requires_live_session_rebuild(&self, next: &Self) -> bool {
            match (self, next) {
                (Self::Nes(before), Self::Nes(after)) => before.video.filter != after.video.filter,
            }
        }
    }
}

pub mod local {
    pub const HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION: u32 = 2;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct HostBackendLocalSettings {
        pub schema_version: u32,
        pub video: VideoSettings,
        pub audio: AudioSettings,
    }

    impl Default for HostBackendLocalSettings {
        fn default() -> Self {
            Self {
                schema_version: HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION,
                video: VideoSettings::default(),
                audio: AudioSettings::default(),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct WindowVideoSettings {
        pub fullscreen_default: bool,
        pub scaling: ScalingMode,
    }

    impl Default for WindowVideoSettings {
        fn default() -> Self {
            Self {
                fullscreen_default: false,
                scaling: ScalingMode::FitToWindow,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct BackendPresentationSettings {
        pub vsync: bool,
    }

    impl Default for BackendPresentationSettings {
        fn default() -> Self {
            Self { vsync: true }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct VideoSettings {
        pub window: WindowVideoSettings,
        pub presentation: BackendPresentationSettings,
    }

    #[derive(Default, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct VideoSettingsDocument {
        #[serde(skip_serializing_if = "Option::is_none")]
        window: Option<WindowVideoSettings>,
        #[serde(skip_serializing_if = "Option::is_none")]
        presentation: Option<BackendPresentationSettings>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fullscreen_default: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scaling: Option<ScalingMode>,
        #[serde(skip_serializing_if = "Option::is_none")]
        vsync: Option<bool>,
    }

    impl serde::Serialize for VideoSettings {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            VideoSettingsDocument {
                window: Some(self.window.clone()),
                presentation: Some(self.presentation.clone()),
                fullscreen_default: None,
                scaling: None,
                vsync: None,
            }
            .serialize(serializer)
        }
    }

    impl<'de> serde::Deserialize<'de> for VideoSettings {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let document = VideoSettingsDocument::deserialize(deserializer)?;
            let mut settings = VideoSettings::default();
            if let Some(fullscreen_default) = document.fullscreen_default {
                settings.window.fullscreen_default = fullscreen_default;
            }
            if let Some(scaling) = document.scaling {
                settings.window.scaling = scaling;
            }
            if let Some(vsync) = document.vsync {
                settings.presentation.vsync = vsync;
            }
            if let Some(window) = document.window {
                settings.window = window;
            }
            if let Some(presentation) = document.presentation {
                settings.presentation = presentation;
            }
            Ok(settings)
        }
    }

    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Default,
        serde::Serialize,
        serde::Deserialize,
        strum::EnumIter,
        strum::Display,
        strum::EnumString,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum ScalingMode {
        #[default]
        #[strum(serialize = "fit")]
        FitToWindow,
        #[strum(serialize = "1")]
        X1,
        #[strum(serialize = "2")]
        X2,
        #[strum(serialize = "3")]
        X3,
        #[strum(serialize = "4")]
        X4,
        #[strum(serialize = "5")]
        X5,
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct AudioSettings {
        pub muted: bool,
        pub master_volume_percent: u8,
        pub sample_rate: u32,
        pub latency_ms: u16,
    }

    impl Default for AudioSettings {
        fn default() -> Self {
            Self {
                muted: false,
                master_volume_percent: 100,
                sample_rate: 48_000,
                latency_ms: 50,
            }
        }
    }
}

pub mod app_state {
    use super::{BTreeMap, PathBuf};

    pub const DESKTOP_APP_STATE_SCHEMA_VERSION: u32 = 2;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct RememberedWindowSize {
        pub width: u32,
        pub height: u32,
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    pub struct DesktopAppState {
        pub schema_version: u32,
        pub last_successful_rom_directory: Option<PathBuf>,
        pub window_sizes: BTreeMap<String, RememberedWindowSize>,
        /// Per-system controller assignments: system_id → [(slot_id, controller_id or None)]
        pub controller_assignments: BTreeMap<String, Vec<(String, Option<String>)>>,
    }

    impl DesktopAppState {
        pub fn window_size(&self, host_backend: &str) -> Option<RememberedWindowSize> {
            self.window_sizes.get(host_backend).copied()
        }

        pub fn set_window_size(
            &mut self,
            host_backend: impl Into<String>,
            size: RememberedWindowSize,
        ) {
            self.window_sizes.insert(host_backend.into(), size);
        }
    }

    impl Default for DesktopAppState {
        fn default() -> Self {
            Self {
                schema_version: DESKTOP_APP_STATE_SCHEMA_VERSION,
                last_successful_rom_directory: None,
                window_sizes: BTreeMap::new(),
                controller_assignments: BTreeMap::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nerust_keyboard::Key;

    use super::{
        app_state::{DESKTOP_APP_STATE_SCHEMA_VERSION, DesktopAppState, RememberedWindowSize},
        input::{ShortcutAction, ShortcutBinding},
        local::{
            HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION, HostBackendLocalSettings, ScalingMode,
        },
        shared::{DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION, DesktopSharedSettings},
    };

    #[test]
    fn defaults_track_current_schema_versions() {
        assert_eq!(
            DesktopSharedSettings::default().schema_version,
            DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            HostBackendLocalSettings::default().schema_version,
            HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            DesktopAppState::default().schema_version,
            DESKTOP_APP_STATE_SCHEMA_VERSION
        );
    }

    #[test]
    fn app_state_tracks_window_sizes_per_host_backend() {
        let mut state = DesktopAppState::default();

        state.set_window_size(
            "tao+wgpu",
            RememberedWindowSize {
                width: 960,
                height: 720,
            },
        );

        assert_eq!(
            state.window_size("tao+wgpu"),
            Some(RememberedWindowSize {
                width: 960,
                height: 720,
            })
        );
        assert_eq!(state.window_size("gtk+opengl"), None);
    }

    #[test]
    fn unbound_shortcut_serializes_stably() {
        let encoded = serde_saphyr::to_string(&ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
        })
        .unwrap();

        assert!(encoded.contains("reset"));
        assert!(encoded.contains("null"));
    }

    #[test]
    fn bound_shortcut_serializes_key_name() {
        let encoded = serde_saphyr::to_string(&ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Key::Space),
        })
        .unwrap();

        assert!(encoded.contains("toggle_pause"));
        assert!(encoded.contains("space"));
    }

    #[test]
    fn local_video_settings_decode_legacy_flat_fields() {
        let decoded: HostBackendLocalSettings = serde_saphyr::from_str(
            r#"
schema_version: 1
video:
  fullscreen_default: true
  scaling: x3
  vsync: false
"#,
        )
        .unwrap();

        assert!(decoded.video.window.fullscreen_default);
        assert_eq!(decoded.video.window.scaling, ScalingMode::X3);
        assert!(!decoded.video.presentation.vsync);
    }

    #[test]
    fn local_video_settings_prefer_nested_fields_over_legacy_flat_fields() {
        let decoded: HostBackendLocalSettings = serde_saphyr::from_str(
            r#"
schema_version: 2
video:
  fullscreen_default: true
  scaling: x3
  vsync: false
  window:
    fullscreen_default: false
    scaling: fit_to_window
  presentation:
    vsync: true
"#,
        )
        .unwrap();

        assert!(!decoded.video.window.fullscreen_default);
        assert_eq!(decoded.video.window.scaling, ScalingMode::FitToWindow);
        assert!(decoded.video.presentation.vsync);
    }
}
