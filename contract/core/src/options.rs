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
