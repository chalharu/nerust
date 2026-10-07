use super::manifest::{AudioExpectation, RomCategory};

#[derive(Debug, Clone, Copy)]
pub struct ValidationOptions {
    pub capture_screenshots: bool,
    pub check_expectations: bool,
}

impl ValidationOptions {
    pub const fn capturing() -> Self {
        Self {
            capture_screenshots: true,
            check_expectations: false,
        }
    }

    pub const fn report() -> Self {
        Self {
            capture_screenshots: true,
            check_expectations: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutionTotals {
    pub frames: u64,
}

#[derive(Debug, Clone)]
pub struct ScreenCheck {
    pub frame: u64,
    pub expected_hash: u64,
    pub actual_hash: u64,
    pub screenshot_png: Option<Vec<u8>>,
}

impl ScreenCheck {
    pub fn passed(&self) -> bool {
        self.expected_hash == self.actual_hash
    }
}

#[derive(Debug, Clone)]
pub struct MemoryCheck {
    pub frame: u64,
    pub address: u32,
    pub expected_value: u8,
    pub actual_value: u8,
    pub expected_open_bus: bool,
    pub actual_open_bus: bool,
}

impl MemoryCheck {
    pub fn passed(&self) -> bool {
        self.expected_open_bus == self.actual_open_bus
            && (self.expected_open_bus || self.expected_value == self.actual_value)
    }
}

#[derive(Debug, Clone)]
pub struct RegisterCheck {
    pub frame: u64,
    pub name: String,
    pub expected_value: u64,
    pub actual_value: u64,
}

impl RegisterCheck {
    pub fn passed(&self) -> bool {
        self.expected_value == self.actual_value
    }
}

#[derive(Debug, Clone)]
pub struct SerialCheck {
    pub frame: u64,
    pub channel: String,
    pub expected_bytes: Vec<u8>,
    pub actual_bytes: Vec<u8>,
}

impl SerialCheck {
    pub fn passed(&self) -> bool {
        self.expected_bytes == self.actual_bytes
    }
}

#[derive(Debug, Clone)]
pub struct LogCheck {
    pub frame: u64,
    pub channel: String,
    pub expected_end: String,
    pub end_found: bool,
    pub allowed_fail: Vec<String>,
    pub fail_names: Vec<String>,
    pub missing_allowed: Vec<String>,
    pub unexpected_fail: Vec<String>,
}

impl LogCheck {
    pub fn passed(&self) -> bool {
        self.end_found && self.missing_allowed.is_empty() && self.unexpected_fail.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct AudioObservation {
    pub sample_rate: u32,
    pub samples: u64,
    pub hash: u64,
    pub expected: Option<AudioExpectation>,
}

#[derive(Debug, Clone)]
pub struct CaseValidation {
    pub case_id: String,
    pub category: RomCategory,
    pub description: String,
    pub rom: String,
    pub frames: u64,
    pub final_screen_hash: u64,
    pub screen_checks: Vec<ScreenCheck>,
    pub memory_checks: Vec<MemoryCheck>,
    pub register_checks: Vec<RegisterCheck>,
    pub serial_checks: Vec<SerialCheck>,
    pub log_checks: Vec<LogCheck>,
    pub audio: AudioObservation,
    pub failures: Vec<String>,
    pub expected_failure: bool,
    /// Graduation prompts. NEVER tolerated (not even under
    /// `expected_failure`): a tracked item that needs human action
    /// must stay loud. Covers both stale directions — a flagged case
    /// with no mismatches, and allow-listed names that no longer fail.
    pub stale_notes: Vec<String>,
}

impl CaseValidation {
    pub fn passed(&self) -> bool {
        self.failures.is_empty() && self.stale_notes.is_empty()
    }

    /// Tracked red: flagged expected-failure with actual mismatches,
    /// no pending graduation prompts, and no never-seen-before log
    /// failures. Tolerated by CI (distinct from pass). Unknown log
    /// FAIL names stay hard even under the flag (legacy per-name
    /// precision: new breakage must be loud).
    pub fn is_expected_failure(&self) -> bool {
        self.expected_failure
            && self.stale_notes.is_empty()
            && !self.failures.is_empty()
            && self
                .log_checks
                .iter()
                .all(|check| check.unexpected_fail.is_empty())
    }
}

#[derive(Debug, Clone)]
pub enum CaseOutcome {
    Completed(Box<CaseValidation>),
    InternalError {
        case_id: String,
        category: RomCategory,
        description: String,
        rom: String,
        message: String,
    },
    /// No factory accepts the ROM (no cores enabled, or no system
    /// match). Eligibility, not failure: ignored in reports and
    /// exits, never counted as failed.
    Skipped {
        case_id: String,
        category: RomCategory,
        description: String,
        rom: String,
        reason: String,
    },
}

impl CaseOutcome {
    pub fn case_id(&self) -> &str {
        match self {
            CaseOutcome::Completed(validation) => &validation.case_id,
            CaseOutcome::InternalError { case_id, .. } => case_id,
            CaseOutcome::Skipped { case_id, .. } => case_id,
        }
    }

    pub fn category(&self) -> RomCategory {
        match self {
            CaseOutcome::Completed(validation) => validation.category,
            CaseOutcome::InternalError { category, .. } => *category,
            CaseOutcome::Skipped { category, .. } => *category,
        }
    }

    pub fn passed(&self) -> bool {
        match self {
            CaseOutcome::Completed(validation) => validation.passed(),
            CaseOutcome::InternalError { .. } => false,
            CaseOutcome::Skipped { .. } => false,
        }
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, CaseOutcome::Skipped { .. })
    }

    pub fn is_expected_failure(&self) -> bool {
        match self {
            CaseOutcome::Completed(validation) => validation.is_expected_failure(),
            _ => false,
        }
    }
}
