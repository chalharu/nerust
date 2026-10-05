mod entry;
mod validation;

use crate::{
    manifest::RomCase,
    results::{CaseOutcome, ValidationOptions},
};

pub fn validate_case(
    factory: &dyn nerust_core_traits::factory::CoreFactory,
    case: &RomCase,
    options: ValidationOptions,
) -> CaseOutcome {
    entry::validate_case(factory, case, options)
}
