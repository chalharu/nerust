mod entry;
mod validation;

use crate::manifest::RomCase;
use crate::results::{CaseOutcome, ValidationOptions};

pub fn validate_case(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    entry::validate_case(case, options)
}
