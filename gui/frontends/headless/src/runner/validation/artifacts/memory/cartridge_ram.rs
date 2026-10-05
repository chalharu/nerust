use super::super::{
    super::{assertions::CartridgeRamAssertion, runtime::ValidationRuntime},
    ValidationArtifacts,
};
use crate::{
    error::RomTestError,
    results::{CartridgeRamCheck, ValidationOptions},
};

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct CartridgeRamArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<CartridgeRamCheck>,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_cartridge_ram_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        assertion: CartridgeRamAssertion,
    ) -> Result<(), RomTestError> {
        let (actual_value, actual_open_bus) = runtime
            .peek_cartridge_ram(assertion.address)
            .ok_or_else(|| {
                RomTestError::InvalidManifest(format!(
                    "ROM case `{case_id}` requested check_cartridge_ram outside cartridge RAM at address 0x{:04X}",
                    assertion.address,
                ))
            })?;
        if options.check_expectations && actual_open_bus != assertion.expect_open_bus {
            self.failures.push(format!(
                "{case_id}: cartridge RAM bus state mismatch at frame {} address 0x{:04X} (expected {}, actual {})",
                assertion.frame,
                assertion.address,
                if assertion.expect_open_bus {
                    "open bus"
                } else {
                    "mapped RAM"
                },
                if actual_open_bus {
                    "open bus"
                } else {
                    "mapped RAM"
                }
            ));
        }
        if options.check_expectations
            && !assertion.expect_open_bus
            && actual_value != assertion.expected_value
        {
            self.failures.push(format!(
                "{case_id}: cartridge RAM mismatch at frame {} address 0x{:04X} (expected 0x{:02X}, actual 0x{:02X})",
                assertion.frame,
                assertion.address,
                assertion.expected_value,
                actual_value,
            ));
        }

        self.memory.cartridge_ram.checks.push(CartridgeRamCheck {
            frame: assertion.frame,
            address: u16::try_from(assertion.address)
                .expect("address range validated before dispatch"),
            expected_value: assertion.expected_value,
            actual_value,
            expected_open_bus: assertion.expect_open_bus,
            actual_open_bus,
        });
        Ok(())
    }
}
