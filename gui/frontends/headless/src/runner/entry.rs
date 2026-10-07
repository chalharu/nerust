use super::validation::runner::ValidationRunner;
use crate::{
    error::RomTestError,
    manifest::{RomCase, read_rom},
    results::{CaseOutcome, ValidationOptions},
};

pub(super) fn validate_case(
    factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
    case: &RomCase,
    options: ValidationOptions,
) -> CaseOutcome {
    match read_rom(case).and_then(|rom_bytes| {
        ValidationRunner::new(factories, case, &rom_bytes, options)?.run_case(case)
    }) {
        Ok(validation) => CaseOutcome::Completed(Box::new(validation)),
        Err(RomTestError::NoMatchingSystem { case_id }) => CaseOutcome::Skipped {
            case_id,
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            reason: if factories.is_empty() {
                "no system cores enabled".to_string()
            } else {
                "no factory accepts this ROM".to_string()
            },
        },
        Err(error) => CaseOutcome::InternalError {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            message: error.to_string(),
        },
    }
}
