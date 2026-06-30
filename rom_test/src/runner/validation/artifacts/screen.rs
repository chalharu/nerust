use super::{super::runtime::ValidationRuntime, ValidationArtifacts};
use crate::{
    error::RomTestError,
    results::{ScreenCheck, ValidationOptions},
};

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
