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

#[cfg(test)]
mod tests {
    use nerust_settings_traits::SystemSettings;

    use super::*;

    fn test_settings() -> NesSettings {
        NesSettings {
            video: NesVideoSettings {
                filter: NesVideoFilter::NtscRgb,
            },
            core: NesCoreSettings {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            },
        }
    }

    #[test]
    fn downcast_ref_works() {
        let settings: Box<dyn SystemSettings> = Box::new(NesSettings::default());
        let nes = settings
            .downcast_ref::<NesSettings>()
            .expect("should downcast");
        assert_eq!(nes.video.filter, NesVideoFilter::NtscComposite);
    }

    #[test]
    fn dyn_clone_preserves_values() {
        let settings: Box<dyn SystemSettings> = Box::new(test_settings());
        let cloned = settings.clone();
        let cloned_nes = cloned
            .downcast_ref::<NesSettings>()
            .expect("cloned should downcast");

        assert_eq!(cloned_nes.video.filter, NesVideoFilter::NtscRgb);
        assert_eq!(
            cloned_nes.core.mmc3_irq_variant,
            Some(Mmc3IrqVariant::Sharp)
        );
    }

    #[test]
    fn requires_live_session_rebuild_detects_filter_change() {
        let a: NesSettings = test_settings();
        let mut b = a.clone();
        b.video.filter = NesVideoFilter::NtscSVideo;

        assert!(a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn requires_live_session_rebuild_ignores_core_change() {
        let a: NesSettings = test_settings();
        let mut b = a.clone();
        b.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Nec);

        assert!(!a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn requires_live_session_rebuild_returns_false_for_non_nes_type() {
        let a: NesSettings = test_settings();

        #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
        struct FakeSettings;
        #[typetag::serde]
        impl SystemSettings for FakeSettings {
            fn requires_live_session_rebuild(&self, _next: &dyn SystemSettings) -> bool {
                unreachable!()
            }
        }

        let b: Box<dyn SystemSettings> = Box::new(FakeSettings);
        assert!(!a.requires_live_session_rebuild(&*b));
    }
}
