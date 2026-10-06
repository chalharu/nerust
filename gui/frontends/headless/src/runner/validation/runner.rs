use super::{artifacts::ValidationArtifacts, runtime::ValidationRuntime};
use crate::{
    error::RomTestError,
    events::{ControllerPad, PadState},
    harness::drive_case,
    manifest::RomCase,
    results::{CaseValidation, ValidationOptions},
};

pub(in crate::runner) struct ValidationRunner {
    case_id: String,
    runtime: ValidationRuntime,
    artifacts: ValidationArtifacts,
    options: ValidationOptions,
}

impl ValidationRunner {
    pub(in crate::runner) fn new(
        factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
        case: &RomCase,
        rom_bytes: &[u8],
        options: ValidationOptions,
    ) -> Result<Self, RomTestError> {
        Ok(Self {
            case_id: case.id.clone(),
            runtime: ValidationRuntime::new(factories, case, rom_bytes)?,
            artifacts: ValidationArtifacts::default(),
            options,
        })
    }

    pub(in crate::runner) fn run_case(
        mut self,
        case: &RomCase,
    ) -> Result<CaseValidation, RomTestError> {
        let totals = drive_case(case, &mut self)?;
        Ok(self
            .artifacts
            .finish(case, &mut self.runtime, totals, self.options))
    }

    pub(in crate::runner::validation) fn run_frame(&mut self) -> Result<(), RomTestError> {
        self.runtime.run_frame()
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.runtime.frame_counter()
    }

    pub(in crate::runner::validation) fn record_screen_assert(
        &mut self,
        frame: u64,
        expected_hash: u64,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_screen_assert(
            &self.case_id,
            &mut self.runtime,
            self.options,
            frame,
            expected_hash,
        )
    }

    pub(in crate::runner::validation) fn record_memory_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
        expected_open_bus: bool,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_memory_assert(
            &self.case_id,
            &self.runtime,
            self.options,
            super::artifacts::memory::ExpectedMemory {
                frame,
                address,
                expected_value,
                expected_open_bus,
            },
        )
    }

    pub(in crate::runner::validation) fn record_registers_assert(
        &mut self,
        frame: u64,
        registers: std::collections::BTreeMap<String, u64>,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_registers_assert(
            &self.case_id,
            &self.runtime,
            self.options,
            super::artifacts::registers::ExpectedRegisters { frame, registers },
        )
    }

    pub(in crate::runner::validation) fn reset_runtime(&mut self) -> Result<(), RomTestError> {
        self.runtime.reset()
    }

    pub(in crate::runner::validation) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: String,
        state: PadState,
    ) -> Result<(), RomTestError> {
        self.runtime.apply_standard_controller(pad, button, state)
    }
}
