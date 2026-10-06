use super::{super::runtime::ValidationRuntime, ValidationArtifacts};
use crate::{
    error::RomTestError,
    factory_adapter::MemoryRead,
    results::{MemoryCheck, ValidationOptions},
};

#[derive(Default)]
pub(super) struct MemoryArtifacts {
    pub(super) memory: MemoryCheckArtifacts,
}

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct MemoryCheckArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<MemoryCheck>,
}

/// Expected memory observation for one assertion. Bundled so the
/// record entry stays under the argument-count lint.
pub(in crate::runner::validation) struct ExpectedMemory {
    pub(in crate::runner::validation) frame: u64,
    pub(in crate::runner::validation) address: usize,
    pub(in crate::runner::validation) expected_value: u8,
    pub(in crate::runner::validation) expected_open_bus: bool,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_memory_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        expected: ExpectedMemory,
    ) -> Result<(), RomTestError> {
        let (actual_value, actual_open_bus) = match runtime.peek_memory(expected.address)? {
            MemoryRead::Mapped(value) => (value, false),
            // No meaningful value on a floating bus; the bus state
            // itself is the observation.
            MemoryRead::OpenBus => (0x00, true),
            MemoryRead::Unmapped => {
                return Err(RomTestError::InvalidManifest(format!(
                    "ROM case `{case_id}` requested check_memory outside mapped memory at address 0x{:04X}",
                    expected.address,
                )));
            }
        };
        if options.check_expectations && actual_open_bus != expected.expected_open_bus {
            self.failures.push(format!(
                "{case_id}: memory bus state mismatch at frame {} address 0x{:04X} (expected {}, actual {})",
                expected.frame,
                expected.address,
                if expected.expected_open_bus {
                    "open bus"
                } else {
                    "mapped RAM"
                },
                if actual_open_bus { "open bus" } else { "mapped RAM" },
            ));
        }
        if options.check_expectations
            && !expected.expected_open_bus
            && actual_value != expected.expected_value
        {
            self.failures.push(format!(
                "{case_id}: memory mismatch at frame {} address 0x{:04X} (expected 0x{:02X}, actual 0x{:02X})",
                expected.frame,
                expected.address,
                expected.expected_value,
                actual_value,
            ));
        }

        self.memory.memory.checks.push(MemoryCheck {
            frame: expected.frame,
            address: u16::try_from(expected.address).expect("assertion addresses are u16"),
            expected_value: expected.expected_value,
            actual_value,
            expected_open_bus: expected.expected_open_bus,
            actual_open_bus,
        });
        Ok(())
    }
}
