use nerust_settings_traits::SystemSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mmc3IrqVariant {
    #[default]
    Sharp,
    Nec,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NesVideoSettings {
    pub filter: NesVideoFilter,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NesCoreSettings {
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NesSettings {
    pub video: NesVideoSettings,
    pub core: NesCoreSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NesVideoFilter {
    None,
    #[default]
    NtscComposite,
    NtscSVideo,
    NtscRgb,
}

#[typetag::serde]
impl SystemSettings for NesSettings {
    fn requires_live_session_rebuild(&self, next: &dyn SystemSettings) -> bool {
        if let Some(other) = next.downcast_ref::<NesSettings>() {
            self.video.filter != other.video.filter
        } else {
            false
        }
    }
}
