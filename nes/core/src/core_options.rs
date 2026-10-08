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
    /// Explicit submapper override for ROMs whose headers cannot name
    /// it (legacy iNES). Applied to the parsed header before mapper
    /// resolution, so all downstream logic sees one truth. Validated
    /// by `CartridgeData` like parsed data.
    #[serde(default)]
    pub submapper: Option<u8>,
}

impl nerust_core_traits::CoreOptions for CoreOptions {}
