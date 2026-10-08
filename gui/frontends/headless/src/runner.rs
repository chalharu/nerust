mod entry;
mod measure;
mod validation;

use crate::{
    manifest::RomCase,
    results::{CaseOutcome, ValidationOptions},
};

pub fn validate_case(
    factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
    case: &RomCase,
    options: ValidationOptions,
) -> CaseOutcome {
    entry::validate_case(factories, case, options)
}

pub use measure::{MeasureOutcome, measure_case};
