pub const HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HostBackendLocalSettings {
    pub schema_version: u32,
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub touch_overlay: TouchOverlaySettings,
    pub screen_orientation: ScreenOrientation,
}

impl Default for HostBackendLocalSettings {
    fn default() -> Self {
        Self {
            schema_version: HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION,
            video: VideoSettings::default(),
            audio: AudioSettings::default(),
            touch_overlay: TouchOverlaySettings::default(),
            screen_orientation: ScreenOrientation::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenOrientation {
    #[default]
    Auto,
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TouchOverlaySettings {
    pub visibility: TouchOverlayVisibility,
    pub opacity_percent: u8,
    pub scale_percent: u8,
    pub vertical_offset_percent: i8,
    pub haptics: bool,
}

impl Default for TouchOverlaySettings {
    fn default() -> Self {
        Self {
            visibility: TouchOverlayVisibility::Auto,
            opacity_percent: 65,
            scale_percent: 100,
            vertical_offset_percent: 0,
            haptics: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchOverlayVisibility {
    Always,
    #[default]
    Auto,
    Hidden,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMode {
    #[default]
    FitToWindow,
    X1,
    X2,
    X3,
    X4,
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
