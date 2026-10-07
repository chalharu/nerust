use super::{super::runtime::ValidationRuntime, ValidationArtifacts};
use crate::{
    manifest::RomCase,
    results::{AudioObservation, CaseValidation, ExecutionTotals, ValidationOptions},
};

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn finish(
        mut self,
        case: &RomCase,
        runtime: &mut ValidationRuntime,
        totals: ExecutionTotals,
        options: ValidationOptions,
    ) -> CaseValidation {
        let final_screen_hash = runtime.screen_hash();
        let audio = AudioObservation {
            sample_rate: runtime.audio_sample_rate(),
            samples: runtime.audio_samples(),
            hash: runtime.audio_hash(),
            expected: case.expected_audio.clone(),
        };

        if options.check_expectations
            && let Some(expected_audio) = &audio.expected
        {
            if audio.samples != expected_audio.samples {
                self.failures.push(format!(
                    "{}: audio sample mismatch (expected {}, actual {})",
                    case.id, expected_audio.samples, audio.samples
                ));
            }
            if audio.hash != expected_audio.hash {
                self.failures.push(format!(
                    "{}: audio hash mismatch (expected 0x{:016X}, actual 0x{:016X})",
                    case.id, expected_audio.hash, audio.hash
                ));
            }
        }

        // No-rot rule: a tracked red that now passes must graduate.
        // Only meaningful when expectations were checked (capture
        // records no mismatches by design).
        let stale_expected_failure =
            case.expected_failure && options.check_expectations && self.failures.is_empty();
        if stale_expected_failure {
            self.failures.push(format!(
                "{}: expected failure went stale (now passing — graduate it by dropping expected_failure)",
                case.id
            ));
        }

        CaseValidation {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            frames: totals.frames,
            final_screen_hash,
            screen_checks: self.screen.screen_checks,
            memory_checks: self.memory.memory.checks,
            register_checks: self.registers.registers.checks,
            serial_checks: self.serial.serial.checks,
            audio,
            failures: self.failures,
            expected_failure: case.expected_failure,
            stale_expected_failure,
        }
    }
}
