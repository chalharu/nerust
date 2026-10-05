use super::validation::runner::ValidationRunner;
use crate::{
    manifest::{RomCase, read_rom},
    results::{CaseOutcome, ValidationOptions},
};

pub(super) fn validate_case(
    factory: &dyn nerust_core_traits::factory::CoreFactory,
    case: &RomCase,
    options: ValidationOptions,
) -> CaseOutcome {
    match read_rom(case).and_then(|rom_bytes| {
        ValidationRunner::new(factory, case, &rom_bytes, options)?.run_case(case)
    }) {
        Ok(validation) => CaseOutcome::Completed(validation),
        Err(error) => CaseOutcome::InternalError {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            message: error.to_string(),
        },
    }
}
