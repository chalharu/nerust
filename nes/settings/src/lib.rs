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

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
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

#[typetag::serde]
impl nerust_settings_traits::SystemSettings for NesSettings {
    fn requires_live_session_rebuild(
        &self,
        next: &dyn nerust_settings_traits::SystemSettings,
    ) -> bool {
        let any: &dyn std::any::Any = next;
        if let Some(other) = any.downcast_ref::<NesSettings>() {
            self.video.filter != other.video.filter
        } else {
            false
        }
    }
}
