use super::{
    error::RomTestError,
    events::{ControllerPad, PadState, RomAssertion, RomEventKind},
    manifest::RomCase,
    results::ExecutionTotals,
};

pub trait CaseHarness {
    fn run_frame(&mut self) -> Result<(), RomTestError>;
    fn frame_counter(&self) -> u64;
    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError>;
    fn on_reset(&mut self) -> Result<(), RomTestError>;
    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: String,
        state: PadState,
    ) -> Result<(), RomTestError>;
}

pub fn drive_case<H: CaseHarness>(
    case: &RomCase,
    harness: &mut H,
) -> Result<ExecutionTotals, RomTestError> {
    let final_frame = case.final_frame();
    let mut next_event = 0_usize;

    dispatch_pending_events(case, harness, &mut next_event)?;

    while harness.frame_counter() < final_frame {
        harness.run_frame()?;
        dispatch_pending_events(case, harness, &mut next_event)?;
    }

    Ok(ExecutionTotals {
        frames: harness.frame_counter(),
    })
}

fn dispatch_pending_events<H: CaseHarness>(
    case: &RomCase,
    harness: &mut H,
    next_event: &mut usize,
) -> Result<(), RomTestError> {
    while let Some(event) = case.events.get(*next_event) {
        if event.frame != harness.frame_counter() {
            break;
        }

        if let Some(assertion) = event.kind.assertion() {
            harness.on_assert(event.frame, &assertion)?;
        } else {
            match &event.kind {
                RomEventKind::Reset => {
                    harness.on_reset()?;
                }
                RomEventKind::StandardController { pad, button, state } => {
                    harness.on_standard_controller(*pad, button.clone(), *state)?;
                }
                RomEventKind::Assert { .. }
                | RomEventKind::CheckScreen { .. }
                | RomEventKind::CheckMemory { .. }
                | RomEventKind::CheckRegisters { .. }
                | RomEventKind::CheckSerial { .. } => unreachable!(),
            }
        }

        *next_event += 1;
    }

    Ok(())
}
