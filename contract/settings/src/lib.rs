use nerust_contract_options::Mmc3IrqVariant;
use nerust_input_schema::SystemId;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DESKTOP_SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct DesktopSettings {
    pub schema_version: u32,
    pub general: GeneralSettings,
    pub paths: PathsSettings,
    pub persistence: PersistenceSettings,
    pub input: InputSettings,
    pub shortcuts: ShortcutSettings,
    pub video: DesktopVideoSettings,
    pub audio: AudioSettings,
    pub host: HostSettings,
    pub systems: BTreeMap<SystemId, SystemSettings>,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            schema_version: DESKTOP_SETTINGS_SCHEMA_VERSION,
            general: GeneralSettings::default(),
            paths: PathsSettings::default(),
            persistence: PersistenceSettings::default(),
            input: InputSettings::default(),
            shortcuts: ShortcutSettings::default(),
            video: DesktopVideoSettings::default(),
            audio: AudioSettings::default(),
            host: HostSettings::default(),
            systems: BTreeMap::new(),
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(default)]
pub struct GeneralSettings {
    pub recent_roms: Vec<PathBuf>,
    pub last_open_directory: Option<PathBuf>,
    pub restore_last_session: bool,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(default)]
pub struct PathsSettings {
    pub rom_library_dirs: Vec<PathBuf>,
    pub default_open_dir: Option<PathBuf>,
    pub screenshot_dir: Option<PathBuf>,
    pub export_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct PersistenceSettings {
    pub storage_policy: StoragePolicy,
    pub state_root: Option<PathBuf>,
    pub mapper_save_root: Option<PathBuf>,
    pub auto_save_on_exit: bool,
    pub auto_load_mapper_save: bool,
}

impl Default for PersistenceSettings {
    fn default() -> Self {
        Self {
            storage_policy: StoragePolicy::RomSidecar,
            state_root: None,
            mapper_save_root: None,
            auto_save_on_exit: true,
            auto_load_mapper_save: true,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum StoragePolicy {
    #[default]
    RomSidecar,
    AppData,
    CustomRoots,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(default)]
pub struct InputSettings {
    pub keyboard_profiles: BTreeMap<SystemId, BindingProfile>,
    pub gamepad_profiles: BTreeMap<SystemId, BindingProfile>,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(default)]
pub struct BindingProfile {
    pub bindings: Vec<ControlBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct ControlBinding {
    pub attachment: PersistedAttachmentId,
    pub control: PersistedControlId,
    pub source: HostInputSource,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde_derive::Serialize,
    serde_derive::Deserialize,
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

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde_derive::Serialize,
    serde_derive::Deserialize,
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

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum HostInputSource {
    Keyboard(KeyboardKey),
    GamepadButton(String),
    GamepadAxis(GamepadAxisSource),
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GamepadAxisSource {
    pub axis: String,
    pub direction: AxisDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Negative,
    Positive,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(default)]
pub struct ShortcutSettings {
    pub keyboard: Vec<ShortcutBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct ShortcutBinding {
    pub action: ShortcutAction,
    pub key: KeyboardKey,
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
    serde_derive::Serialize,
    serde_derive::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    TogglePause,
    Reset,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectNextSlot,
    SelectPreviousSlot,
    ToggleFullscreen,
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
    serde_derive::Serialize,
    serde_derive::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardKey {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Enter,
    Escape,
    Space,
    Tab,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

#[derive(Debug, Clone, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct DesktopVideoSettings {
    pub scale_mode: ScaleMode,
    pub integer_scale: bool,
    pub fullscreen: bool,
    pub preserve_aspect_ratio: bool,
    pub vsync: bool,
    pub renderer_preference: RendererPreference,
}

impl Default for DesktopVideoSettings {
    fn default() -> Self {
        Self {
            scale_mode: ScaleMode::FitWindow,
            integer_scale: false,
            fullscreen: false,
            preserve_aspect_ratio: true,
            vsync: true,
            renderer_preference: RendererPreference::Auto,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    #[default]
    FitWindow,
    FillWindow,
    Fixed,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RendererPreference {
    #[default]
    Auto,
    OpenGl,
    Wgpu,
}

#[derive(Debug, Clone, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub latency_ms: u32,
    pub master_volume: f32,
    pub muted: bool,
    pub output_device_id: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            buffer_size: 128,
            latency_ms: 20,
            master_volume: 1.0,
            muted: false,
            output_device_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct HostSettings {
    pub remember_window_bounds: bool,
    pub pause_on_focus_loss: bool,
    pub clear_input_on_focus_loss: bool,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            remember_window_bounds: true,
            pause_on_focus_loss: false,
            clear_input_on_focus_loss: true,
            window_width: None,
            window_height: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(tag = "system", content = "settings", rename_all = "snake_case")]
pub enum SystemSettings {
    Nes(NesSettings),
}

#[derive(Debug, Clone, PartialEq, Default, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct NesSettings {
    pub core: NesCoreSettings,
    pub video: NesVideoSettings,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(default)]
pub struct NesCoreSettings {
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(default)]
pub struct NesVideoSettings {
    pub filter: NesVideoFilter,
}

impl Default for NesVideoSettings {
    fn default() -> Self {
        Self {
            filter: NesVideoFilter::NtscComposite,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde_derive::Serialize, serde_derive::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum NesVideoFilter {
    None,
    NtscRgb,
    #[default]
    NtscComposite,
    NtscSVideo,
}

#[cfg(test)]
mod tests {
    use super::{
        AudioSettings, DESKTOP_SETTINGS_SCHEMA_VERSION, DesktopSettings, KeyboardKey,
        ShortcutAction, ShortcutBinding,
    };

    #[test]
    fn desktop_settings_default_uses_current_schema_version() {
        let settings = DesktopSettings::default();

        assert_eq!(settings.schema_version, DESKTOP_SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.audio, AudioSettings::default());
    }

    #[test]
    fn shortcut_bindings_serialize_stably() {
        let encoded = serde_yaml::to_string(&ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: KeyboardKey::Space,
        })
        .unwrap();

        assert!(encoded.contains("toggle_pause"));
        assert!(encoded.contains("space"));
    }
}
