// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::error::RomTestError;
use super::events::{ButtonCode, ControllerPad, PadState, RomAssertion, RomEventKind};
use super::manifest::RomCase;
use super::results::ExecutionTotals;
use crate::core_api::Buttons;

pub trait CaseHarness {
    fn run_frame(&mut self) -> u64;
    fn frame_counter(&self) -> u64;
    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError>;
    fn on_reset(&mut self) -> Result<(), RomTestError>;
    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), RomTestError>;
    fn on_microphone(&mut self, state: PadState) -> Result<(), RomTestError>;
}

pub fn drive_case<H: CaseHarness>(
    case: &RomCase,
    harness: &mut H,
) -> Result<ExecutionTotals, RomTestError> {
    let final_frame = case.final_frame();
    let mut total_steps = 0_u64;
    let mut next_event = 0_usize;

    dispatch_pending_events(case, harness, &mut next_event)?;

    while harness.frame_counter() < final_frame {
        total_steps += harness.run_frame();
        dispatch_pending_events(case, harness, &mut next_event)?;
    }

    Ok(ExecutionTotals {
        frames: harness.frame_counter(),
        steps: total_steps,
    })
}

pub(crate) fn apply_button_state(current: Buttons, button: Buttons, state: PadState) -> Buttons {
    match state {
        PadState::Pressed => current | button,
        PadState::Released => current & !button,
    }
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
            match event.kind {
                RomEventKind::Reset => {
                    harness.on_reset()?;
                }
                RomEventKind::StandardController { pad, button, state } => {
                    harness.on_standard_controller(pad, button, state)?;
                }
                RomEventKind::Microphone { state } => {
                    harness.on_microphone(state)?;
                }
                RomEventKind::Assert { .. }
                | RomEventKind::CheckScreen { .. }
                | RomEventKind::CheckWorkRam { .. }
                | RomEventKind::CheckCartridgeRam { .. }
                | RomEventKind::CheckPpuVram { .. } => unreachable!(),
            }
        }

        *next_event += 1;
    }

    Ok(())
}
