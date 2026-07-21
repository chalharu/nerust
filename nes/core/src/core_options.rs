#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mmc3IrqVariant {
    #[default]
    Sharp,
    Nec,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoreOptions {
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
}

impl CoreOptions {
    pub fn into_bytes(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(self).expect("CoreOptions serialization should not fail")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        rmp_serde::from_slice(bytes).map_err(|e| e.to_string())
    }
}

impl nerust_core_traits::CoreOptions for CoreOptions {}
