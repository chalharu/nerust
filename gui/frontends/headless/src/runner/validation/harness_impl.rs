use super::runner::ValidationRunner;
use crate::{error::RomTestError, events::RomAssertion, harness::CaseHarness};

impl CaseHarness for ValidationRunner {
    fn run_frame(&mut self) -> Result<(), RomTestError> {
        self.run_frame()
    }

    fn frame_counter(&self) -> u64 {
        self.frame_counter()
    }

    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError> {
        // Intentionally exhaustive (no wildcard arm): a new
        // `RomAssertion` variant fails to build until handled here.
        // No dispatcher registry: silent pass-through is worse.
        match assertion {
            RomAssertion::Screen { hash } => self.record_screen_assert(frame, *hash),
            RomAssertion::Memory {
                address,
                value,
                open_bus,
            } => self.record_memory_assert(frame, *address as usize, *value, *open_bus),
            RomAssertion::Registers { registers } => {
                self.record_registers_assert(frame, registers.clone())
            }
            RomAssertion::Serial { channel, bytes } => {
                self.record_serial_assert(frame, channel.clone(), bytes.clone())
            }
            RomAssertion::Log {
                channel,
                end,
                fail_prefix,
                allowed_fail,
            } => self.record_log_assert(
                frame,
                channel.clone(),
                end.clone(),
                fail_prefix.clone(),
                allowed_fail.clone(),
            ),
        }
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.reset_runtime()
    }

    fn on_standard_controller(
        &mut self,
        pad: crate::events::ControllerPad,
        button: String,
        state: crate::events::PadState,
    ) -> Result<(), RomTestError> {
        self.apply_standard_controller(pad, button, state)
    }
}
