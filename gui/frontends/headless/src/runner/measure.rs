//! Benchmark driving through the same [`CaseHarness`] seam as
//! validation: one driving site ([`drive_case`]), two harness modes.
//! `perf` owns only CLI parsing and timing aggregation; stepping
//! semantics live here, next to validation, so the two cannot drift.

use crate::{
    error::RomTestError,
    events::{ControllerPad, PadState, RomAssertion},
    factory_adapter,
    harness::{CaseHarness, drive_case},
    manifest::RomCase,
};

/// Outcome of one measured run: frames/steps driven plus the rolling
/// screen checksum as a determinism marker.
pub struct MeasureOutcome {
    pub frames: u64,
    pub steps: u64,
    pub final_marker: u64,
}

/// Drive `case` to its final frame, hashing published screens and
/// discarding audio. Asserts are inputs here, not checks: measurement
/// observes the shipped execution path, including pad/input dispatch
/// and reset handling, exactly as validation drives it.
pub fn measure_case(
    factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
    case: &RomCase,
    rom_bytes: &[u8],
) -> Result<MeasureOutcome, RomTestError> {
    let mut runtime = MeasureRuntime::new(factories, case, rom_bytes)?;
    let totals = drive_case(case, &mut runtime)?;
    Ok(MeasureOutcome {
        frames: totals.frames,
        steps: runtime.steps,
        final_marker: runtime.checksum,
    })
}

struct MeasureRuntime {
    system: factory_adapter::TestSystem,
    checksum: u64,
    frame_counter: u64,
    steps: u64,
}

impl MeasureRuntime {
    fn new(
        factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        Ok(Self {
            system: factory_adapter::open_headless_system(
                factories,
                &case.id,
                rom_bytes,
                case.options.clone(),
                case.audio_sample_rate(),
            )?,
            checksum: 0,
            frame_counter: 0,
            steps: 0,
        })
    }
}

impl CaseHarness for MeasureRuntime {
    fn run_frame(&mut self) -> Result<(), RomTestError> {
        // Same stepping semantics as validation (`step_frame` per
        // frame); only the observation differs (checksum, no asserts).
        self.system.control().step_frame()?;
        let mut observe = self.system.observe();
        // Discard tapped samples: perf hashes screens only. Without a
        // drain the tap would grow unbounded over rounds x cases.
        drop(observe.drain_audio()?);
        // Channel taps would likewise grow unbounded: no cumulative
        // log exists here, so drop each frame's delta.
        for channel in observe.channel_names() {
            drop(observe.drain_channel(channel)?);
        }
        // Per-frame checksum over published palette bytes.
        for &b in observe.screen_buffer().as_ref() {
            self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(b));
        }
        self.frame_counter += 1;
        self.steps += 1;
        Ok(())
    }

    fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    fn on_assert(&mut self, _frame: u64, _assertion: &RomAssertion) -> Result<(), RomTestError> {
        Ok(())
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.system.control().reset()?;
        // Post-reset taps hold pre-reset deltas the core rebuilt
        // around: drop them like validation does, so the next frames
        // measure fresh streams.
        let observe = self.system.observe();
        for channel in observe.channel_names() {
            drop(observe.drain_channel(channel)?);
        }
        Ok(())
    }

    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: String,
        state: PadState,
    ) -> Result<(), RomTestError> {
        self.system
            .control()
            .set_button(pad.index(), &button, matches!(state, PadState::Pressed))
    }
}
