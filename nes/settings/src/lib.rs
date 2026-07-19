#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mmc3IrqVariant {
    #[default]
    Sharp,
    Nec,
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

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NesSettings {
    pub video: NesVideoSettings,
    pub core: NesCoreSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NesVideoFilter {
    None,
    #[default]
    NtscComposite,
    NtscSVideo,
    NtscRgb,
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
