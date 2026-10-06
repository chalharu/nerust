use std::collections::BTreeMap;

use super::{super::runtime::ValidationRuntime, ValidationArtifacts};
use crate::{
    error::RomTestError,
    results::{RegisterCheck, ValidationOptions},
};

#[derive(Default)]
pub(super) struct RegisterArtifacts {
    pub(super) registers: RegisterCheckArtifacts,
}

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct RegisterCheckArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<RegisterCheck>,
}

/// Expected register observation for one assertion. Only listed names
/// compare; extras the core exposes are ignored. Unknown names are
/// manifest typos and fail loudly, mirroring button/memory handling.
pub(in crate::runner::validation) struct ExpectedRegisters {
    pub(in crate::runner::validation) frame: u64,
    pub(in crate::runner::validation) registers: BTreeMap<String, u64>,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_registers_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        expected: ExpectedRegisters,
    ) -> Result<(), RomTestError> {
        let actual = runtime.peek_registers()?;
        for (name, expected_value) in &expected.registers {
            let Some((_, actual_value)) = actual.iter().find(|(n, _)| *n == name) else {
                return Err(RomTestError::InvalidManifest(format!(
                    "ROM case `{case_id}` requested check_registers for unknown register `{name}`",
                )));
            };
            if options.check_expectations && *actual_value != *expected_value {
                self.failures.push(format!(
                    "{case_id}: register mismatch at frame {} `{name}` (expected 0x{expected_value:X}, actual 0x{actual_value:X})",
                    expected.frame,
                ));
            }
            self.registers.registers.checks.push(RegisterCheck {
                frame: expected.frame,
                name: name.clone(),
                expected_value: *expected_value,
                actual_value: *actual_value,
            });
        }
        Ok(())
    }
}
