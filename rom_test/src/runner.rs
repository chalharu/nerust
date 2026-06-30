mod entry;
mod validation;

use crate::{
    manifest::RomCase,
    results::{CaseOutcome, ValidationOptions},
};

pub fn validate_case(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    entry::validate_case(case, options)
}
