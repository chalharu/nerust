use super::ValidationArtifacts;
use crate::{
    error::RomTestError,
    factory_adapter::FrameObserve,
    media::{encode_screenshot_png, screen_hash},
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
        mut observe: FrameObserve,
        options: ValidationOptions,
        frame: u64,
        expected_hash: u64,
    ) -> Result<(), RomTestError> {
        let actual_hash = screen_hash(observe.screen_buffer());
        if options.check_expectations && actual_hash != expected_hash {
            self.failures.push(format!(
                "{case_id}: screen hash mismatch at frame {frame} (expected 0x{expected_hash:016X}, actual 0x{actual_hash:016X})",
            ));
        }

        let screenshot_png = if options.capture_screenshots {
            Some(encode_screenshot_png(observe.screen_buffer())?)
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
