pub mod field;

use nerust_settings_traits::SystemSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GbaSettings;

#[typetag::serde]
impl SystemSettings for GbaSettings {
    fn requires_live_session_rebuild(&self, _next: &dyn SystemSettings) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_traits::SystemSettings;

    use super::*;

    #[test]
    fn default_settings() {
        let settings = GbaSettings;
        assert!(!settings.requires_live_session_rebuild(&GbaSettings));
    }

    #[test]
    fn dyn_clone_preserves_values() {
        let settings: Box<dyn SystemSettings> = Box::new(GbaSettings);
        let cloned = settings.clone();
        let cloned_gba = cloned
            .downcast_ref::<GbaSettings>()
            .expect("cloned should downcast");
        assert_eq!(cloned_gba, &GbaSettings);
    }
}
