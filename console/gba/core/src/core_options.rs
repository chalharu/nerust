use nerust_core_traits::CoreOptions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GbaCoreOptions {
    /// Boktai solar-sensor light level (0 = blinding .. 0xFF = dark).
    pub solar_light_level: u8,
}

impl Default for GbaCoreOptions {
    fn default() -> Self {
        Self {
            solar_light_level: 0x60,
        }
    }
}

impl CoreOptions for GbaCoreOptions {}
