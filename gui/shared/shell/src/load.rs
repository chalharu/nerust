use nerust_contract_options::{CoreOptions, Mmc3IrqVariant};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NesLoadOptions {
    pub mmc3_irq_variant: Option<NesMmc3IrqVariant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NesMmc3IrqVariant {
    Sharp,
    Nec,
}

impl NesLoadOptions {
    pub fn into_core_options(self) -> CoreOptions {
        CoreOptions {
            mmc3_irq_variant: self.mmc3_irq_variant.map(|variant| match variant {
                NesMmc3IrqVariant::Sharp => Mmc3IrqVariant::Sharp,
                NesMmc3IrqVariant::Nec => Mmc3IrqVariant::Nec,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NesLoadOptions, NesMmc3IrqVariant};
    use nerust_contract_options::{CoreOptions, Mmc3IrqVariant};

    #[test]
    fn nes_load_options_translate_to_core_options() {
        assert_eq!(
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Sharp),
            }
            .into_core_options(),
            CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            }
        );
    }
}
