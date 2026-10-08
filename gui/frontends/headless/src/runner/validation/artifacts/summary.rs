use super::{AudioSnapshot, ValidationArtifacts};
use crate::{
    factory_adapter::FrameObserve,
    manifest::RomCase,
    media::screen_hash,
    results::{AudioObservation, CaseValidation, ExecutionTotals, ValidationOptions},
};

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn finish(
        mut self,
        case: &RomCase,
        mut observe: FrameObserve,
        audio: AudioSnapshot,
        totals: ExecutionTotals,
        options: ValidationOptions,
        system: &'static str,
    ) -> CaseValidation {
        let final_screen_hash = screen_hash(observe.screen_buffer());
        let audio = AudioObservation {
            sample_rate: audio.sample_rate,
            samples: audio.samples,
            hash: audio.hash,
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
        // records no mismatches by design). Stale prompts bypass
        // tolerance unconditionally: a fix must never stay silent.
        let mut stale_notes = std::mem::take(&mut self.stale_pending);
        if case.expected_failure && options.check_expectations && self.failures.is_empty() {
            stale_notes.push(format!(
                "{}: expected failure went stale (now passing — graduate it by dropping expected_failure)",
                case.id
            ));
        }

        CaseValidation {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            system,
            // Entry stamps the wall-clock elapsed after the run;
            // finish only sees drive totals, never wall time.
            elapsed: std::time::Duration::ZERO,
            frames: totals.frames,
            final_screen_hash,
            screen_checks: self.screen.screen_checks,
            memory_checks: self.memory.memory.checks,
            register_checks: self.registers.registers.checks,
            serial_checks: self.serial.serial.checks,
            log_checks: self.log.log.checks,
            audio,
            failures: self.failures,
            expected_failure: case.expected_failure,
            stale_notes,
        }
    }
}
