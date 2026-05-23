// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(
    unused_imports,
    reason = "different harness targets reuse this facade with different subsets of the shared API"
)]

mod core_api;
pub mod error;
pub mod events;
pub mod harness;
pub mod manifest;
mod media;
pub mod perf;
pub mod report;
pub mod results;
pub mod runner;
mod screen_api;
mod serde_helpers;
#[cfg(test)]
mod tests;

pub use self::error::RomTestError;
pub use self::events::{
    ButtonCode, ControllerPad, MemoryAssertionSpace, PadState, RomAssertion, RomEvent, RomEventKind,
};
pub use self::harness::{CaseHarness, drive_case};
pub use self::manifest::{
    AudioExpectation, DEFAULT_AUDIO_SAMPLE_RATE, RomCase, RomCategory, RomManifest,
    default_manifest_path, load_default_manifest, load_manifest, read_rom,
};
pub use self::report::{ReportSummary, default_output_root, write_html_report};
pub use self::results::{
    AudioObservation, CartridgeRamCheck, CaseOutcome, CaseValidation, ExecutionTotals,
    PpuVramCheck, ScreenCheck, ValidationOptions, WorkRamCheck,
};
pub use self::runner::validate_case;

#[cfg(test)]
pub(crate) use self::manifest::apply_case_rom_overrides;
