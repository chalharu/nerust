// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::runtime::ValidationRuntime;
use super::ValidationArtifacts;
use crate::error::RomTestError;
use crate::results::{ScreenCheck, ValidationOptions};

#[derive(Default)]
pub(super) struct ScreenArtifacts {
    pub(super) screen_checks: Vec<ScreenCheck>,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_screen_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        frame: u64,
        expected_hash: u64,
    ) -> Result<(), RomTestError> {
        let actual_hash = runtime.screen_hash();
        if options.check_expectations && actual_hash != expected_hash {
            self.failures.push(format!(
                "{case_id}: screen hash mismatch at frame {frame} (expected 0x{expected_hash:016X}, actual 0x{actual_hash:016X})",
            ));
        }

        let screenshot_png = if options.capture_screenshots {
            Some(runtime.capture_screenshot_png()?)
        } else {
            None
        };

        self.screen.screen_checks.push(ScreenCheck {
            frame,
            expected_hash,
            actual_hash,
            screenshot_png,
        });
        Ok(())
    }
}
