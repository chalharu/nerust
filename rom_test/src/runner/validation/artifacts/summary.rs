// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::runtime::ValidationRuntime;
use super::ValidationArtifacts;
use crate::manifest::RomCase;
use crate::results::{AudioObservation, CaseValidation, ExecutionTotals, ValidationOptions};

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn finish(
        mut self,
        case: &RomCase,
        runtime: &ValidationRuntime,
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

        CaseValidation {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            frames: totals.frames,
            steps: totals.steps,
            final_screen_hash,
            screen_checks: self.screen.screen_checks,
            work_ram_checks: self.memory.work_ram.checks,
            cartridge_ram_checks: self.memory.cartridge_ram.checks,
            ppu_vram_checks: self.memory.ppu_vram.checks,
            audio,
            failures: self.failures,
        }
    }
}
