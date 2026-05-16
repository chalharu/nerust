// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crc::{CRC_64_XZ, Crc, Digest};
use nerust_core::Core;
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::LogicalSize;
use nerust_sound_traits::MixerInput;
use png::{BitDepth, ColorType, Encoder};
use serde::de::{self, Visitor};
use serde_derive::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);
pub const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, thiserror::Error)]
pub enum RomTestError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse YAML manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid ROM manifest: {0}")]
    InvalidManifest(String),
    #[error("failed to construct emulator core for {case_id}: {message}")]
    CoreConstruction { case_id: String, message: String },
    #[error("failed to encode screenshot: {0}")]
    ScreenshotEncoding(#[from] png::EncodingError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RomManifest {
    #[serde(default = "default_rom_root")]
    pub rom_root: PathBuf,
    pub cases: Vec<RomCase>,
}

impl RomManifest {
    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.cases.is_empty() {
            return Err(RomTestError::InvalidManifest(
                "manifest must define at least one ROM case".to_string(),
            ));
        }

        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if !ids.insert(case.id.clone()) {
                return Err(RomTestError::InvalidManifest(format!(
                    "duplicate ROM case id `{}`",
                    case.id
                )));
            }
            case.validate()?;
        }

        Ok(())
    }

    pub fn case(&self, id: &str) -> Option<&RomCase> {
        self.cases.iter().find(|case| case.id == id)
    }

    pub fn select<'a>(
        &'a self,
        ids: &[String],
        perf_only: bool,
    ) -> Result<Vec<&'a RomCase>, RomTestError> {
        let mut selected = self
            .cases
            .iter()
            .filter(|case| (!perf_only || case.perf) && (ids.is_empty() || ids.contains(&case.id)))
            .collect::<Vec<_>>();
        selected.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then_with(|| left.id.cmp(&right.id))
        });

        if selected.is_empty() {
            let scope = if perf_only { "perf-enabled " } else { "" };
            let description = if ids.is_empty() {
                "all cases".to_string()
            } else {
                ids.join(", ")
            };
            return Err(RomTestError::InvalidManifest(format!(
                "no {scope}ROM cases matched {description}"
            )));
        }

        Ok(selected)
    }

    fn resolve_paths(&mut self, manifest_path: &Path) {
        let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let resolved_rom_root = if self.rom_root.is_absolute() {
            self.rom_root.clone()
        } else {
            manifest_dir.join(&self.rom_root)
        };

        for case in &mut self.cases {
            case.resolve_rom_path(&resolved_rom_root);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RomCase {
    pub id: String,
    pub category: RomCategory,
    pub description: String,
    pub rom: String,
    #[serde(default)]
    pub perf: bool,
    pub events: Vec<RomEvent>,
    #[serde(default)]
    pub expected_audio: Option<AudioExpectation>,
    #[serde(skip, default)]
    resolved_rom_path: PathBuf,
}

impl RomCase {
    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.id.trim().is_empty() {
            return Err(RomTestError::InvalidManifest(
                "ROM case id must not be empty".to_string(),
            ));
        }
        if self.rom.trim().is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` must define a ROM path",
                self.id
            )));
        }
        if self.description.trim().is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` must define a description",
                self.id
            )));
        }
        let rom_path = self.resolved_rom_path()?;
        if !rom_path.is_file() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` references missing ROM `{}`",
                self.id,
                rom_path.display()
            )));
        }
        if self.events.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` must define at least one event",
                self.id
            )));
        }

        let mut last_frame = 0_u64;
        for (index, event) in self.events.iter().enumerate() {
            event.validate(&self.id)?;
            if index > 0 && event.frame < last_frame {
                return Err(RomTestError::InvalidManifest(format!(
                    "ROM case `{}` has out-of-order event at frame {}",
                    self.id, event.frame
                )));
            }
            last_frame = event.frame;
        }

        if let Some(expected_audio) = &self.expected_audio {
            expected_audio.validate(&self.id)?;
        }

        Ok(())
    }

    pub fn final_frame(&self) -> u64 {
        self.events.last().map(|event| event.frame).unwrap_or(0)
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.expected_audio
            .as_ref()
            .map_or(DEFAULT_AUDIO_SAMPLE_RATE, |expected| expected.sample_rate)
    }

    fn resolve_rom_path(&mut self, rom_root: &Path) {
        self.resolved_rom_path = rom_root.join(&self.rom);
    }

    fn resolved_rom_path(&self) -> Result<&Path, RomTestError> {
        if self.resolved_rom_path.as_os_str().is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` does not have a resolved ROM path",
                self.id
            )));
        }

        Ok(&self.resolved_rom_path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RomCategory {
    Cpu,
    Ppu,
    Apu,
    Mapper,
    Input,
}

impl RomCategory {
    pub const fn label(self) -> &'static str {
        match self {
            RomCategory::Cpu => "CPU Tests",
            RomCategory::Ppu => "PPU Tests",
            RomCategory::Apu => "APU Tests",
            RomCategory::Mapper => "Mapper-specific Tests",
            RomCategory::Input => "Input Tests",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RomEvent {
    pub frame: u64,
    #[serde(flatten)]
    pub kind: RomEventKind,
}

impl RomEvent {
    fn validate(&self, _case_id: &str) -> Result<(), RomTestError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RomEventKind {
    CheckScreen {
        #[serde(with = "hex_u64")]
        hash: u64,
    },
    Reset,
    StandardController {
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerPad {
    Pad1,
    Pad2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "matches existing NES button names"
)]
pub enum ButtonCode {
    A,
    B,
    SELECT,
    START,
    UP,
    DOWN,
    LEFT,
    RIGHT,
}

impl From<ButtonCode> for Buttons {
    fn from(value: ButtonCode) -> Self {
        match value {
            ButtonCode::A => Buttons::A,
            ButtonCode::B => Buttons::B,
            ButtonCode::SELECT => Buttons::SELECT,
            ButtonCode::START => Buttons::START,
            ButtonCode::UP => Buttons::UP,
            ButtonCode::DOWN => Buttons::DOWN,
            ButtonCode::LEFT => Buttons::LEFT,
            ButtonCode::RIGHT => Buttons::RIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioExpectation {
    pub sample_rate: u32,
    pub samples: u64,
    #[serde(with = "hex_u64")]
    pub hash: u64,
}

impl AudioExpectation {
    fn validate(&self, case_id: &str) -> Result<(), RomTestError> {
        if self.sample_rate == 0 {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{case_id}` must not use an audio sample rate of 0"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationOptions {
    pub capture_screenshots: bool,
    pub check_expectations: bool,
}

impl ValidationOptions {
    pub const fn validating() -> Self {
        Self {
            capture_screenshots: false,
            check_expectations: true,
        }
    }

    pub const fn capturing() -> Self {
        Self {
            capture_screenshots: true,
            check_expectations: false,
        }
    }

    pub const fn report() -> Self {
        Self {
            capture_screenshots: true,
            check_expectations: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutionTotals {
    pub frames: u64,
    pub steps: u64,
}

pub trait CaseHarness {
    fn run_frame(&mut self) -> u64;
    fn frame_counter(&self) -> u64;
    fn on_check_screen(&mut self, frame: u64, expected_hash: u64) -> Result<(), RomTestError>;
    fn on_reset(&mut self) -> Result<(), RomTestError>;
    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), RomTestError>;
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

#[derive(Debug, Clone)]
pub struct ScreenCheck {
    pub frame: u64,
    pub expected_hash: u64,
    pub actual_hash: u64,
    pub screenshot_png: Option<Vec<u8>>,
}

impl ScreenCheck {
    pub fn passed(&self) -> bool {
        self.expected_hash == self.actual_hash
    }
}

#[derive(Debug, Clone)]
pub struct AudioObservation {
    pub sample_rate: u32,
    pub samples: u64,
    pub hash: u64,
    pub expected: Option<AudioExpectation>,
}

#[derive(Debug, Clone)]
pub struct CaseValidation {
    pub case_id: String,
    pub category: RomCategory,
    pub description: String,
    pub rom: String,
    pub frames: u64,
    pub steps: u64,
    pub final_screen_hash: u64,
    pub screen_checks: Vec<ScreenCheck>,
    pub audio: AudioObservation,
    pub failures: Vec<String>,
}

impl CaseValidation {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum CaseOutcome {
    Completed(CaseValidation),
    InternalError {
        case_id: String,
        category: RomCategory,
        description: String,
        rom: String,
        message: String,
    },
}

impl CaseOutcome {
    pub fn case_id(&self) -> &str {
        match self {
            CaseOutcome::Completed(validation) => &validation.case_id,
            CaseOutcome::InternalError { case_id, .. } => case_id,
        }
    }

    pub fn category(&self) -> RomCategory {
        match self {
            CaseOutcome::Completed(validation) => validation.category,
            CaseOutcome::InternalError { category, .. } => *category,
        }
    }

    pub fn passed(&self) -> bool {
        match self {
            CaseOutcome::Completed(validation) => validation.passed(),
            CaseOutcome::InternalError { .. } => false,
        }
    }
}

pub struct ValidationRunner {
    case_id: String,
    screen_buffer: ScreenBuffer,
    core: Core,
    controller: StandardController,
    mixer: HashingMixer,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    screen_checks: Vec<ScreenCheck>,
    failures: Vec<String>,
    options: ValidationOptions,
}

impl ValidationRunner {
    pub fn new(
        case: &RomCase,
        rom_bytes: &[u8],
        options: ValidationOptions,
    ) -> Result<Self, RomTestError> {
        let mut input = rom_bytes.iter().copied();
        let core = Core::new(&mut input).map_err(|error| RomTestError::CoreConstruction {
            case_id: case.id.clone(),
            message: error.to_string(),
        })?;

        Ok(Self {
            case_id: case.id.clone(),
            screen_buffer: ScreenBuffer::new(
                FilterType::None,
                LogicalSize {
                    width: 256,
                    height: 240,
                },
            ),
            core,
            controller: StandardController::new(),
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            screen_checks: Vec::new(),
            failures: Vec::new(),
            options,
        })
    }

    pub fn run_case(mut self, case: &RomCase) -> Result<CaseValidation, RomTestError> {
        let totals = drive_case(case, &mut self)?;
        let final_screen_hash = screen_hash(&self.screen_buffer);
        let audio = AudioObservation {
            sample_rate: self.mixer.sample_rate(),
            samples: self.mixer.samples(),
            hash: self.mixer.checksum(),
            expected: case.expected_audio.clone(),
        };

        if self.options.check_expectations {
            if let Some(expected_audio) = &audio.expected {
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
        }

        Ok(CaseValidation {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            frames: totals.frames,
            steps: totals.steps,
            final_screen_hash,
            screen_checks: self.screen_checks,
            audio,
            failures: self.failures,
        })
    }
}

impl CaseHarness for ValidationRunner {
    fn run_frame(&mut self) -> u64 {
        let steps = self.core.run_frame(
            &mut self.screen_buffer,
            &mut self.controller,
            &mut self.mixer,
        );
        self.frame_counter += 1;
        steps
    }

    fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    fn on_check_screen(&mut self, frame: u64, expected_hash: u64) -> Result<(), RomTestError> {
        let actual_hash = screen_hash(&self.screen_buffer);
        if self.options.check_expectations && actual_hash != expected_hash {
            self.failures.push(format!(
                "{}: screen hash mismatch at frame {} (expected 0x{:016X}, actual 0x{:016X})",
                self.case_id, frame, expected_hash, actual_hash
            ));
        }

        let screenshot_png = if self.options.capture_screenshots {
            Some(encode_screenshot_png(&self.screen_buffer)?)
        } else {
            None
        };

        self.screen_checks.push(ScreenCheck {
            frame,
            expected_hash,
            actual_hash,
            screenshot_png,
        });
        Ok(())
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.core.reset();
        Ok(())
    }

    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), RomTestError> {
        let buttons = Buttons::from(button);
        match pad {
            ControllerPad::Pad1 => {
                self.pad1 = apply_button_state(self.pad1, buttons, state);
                self.controller.set_pad1(self.pad1);
            }
            ControllerPad::Pad2 => {
                self.pad2 = apply_button_state(self.pad2, buttons, state);
                self.controller.set_pad2(self.pad2);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ReportSummary {
    pub report_path: PathBuf,
    pub passed: usize,
    pub failed: usize,
}

pub fn default_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml")
}

pub fn default_output_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/rom-tests")
}

pub fn load_manifest(path: &Path) -> Result<RomManifest, RomTestError> {
    let manifest_source = fs::read_to_string(path).map_err(|source| RomTestError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = serde_yaml::from_str::<RomManifest>(&manifest_source).map_err(|source| {
        RomTestError::ParseManifest {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let mut manifest = manifest;
    manifest.resolve_paths(path);
    manifest.validate()?;
    Ok(manifest)
}

pub fn load_default_manifest() -> Result<RomManifest, RomTestError> {
    load_manifest(&default_manifest_path())
}

pub fn read_rom(case: &RomCase) -> Result<Vec<u8>, RomTestError> {
    let rom_path = case.resolved_rom_path()?.to_path_buf();
    fs::read(&rom_path).map_err(|source| RomTestError::ReadFile {
        path: rom_path,
        source,
    })
}

pub fn validate_case(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    match read_rom(case)
        .and_then(|rom_bytes| ValidationRunner::new(case, &rom_bytes, options)?.run_case(case))
    {
        Ok(validation) => CaseOutcome::Completed(validation),
        Err(error) => CaseOutcome::InternalError {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            message: error.to_string(),
        },
    }
}

pub fn write_html_report(
    output_dir: &Path,
    title: &str,
    outcomes: &[CaseOutcome],
) -> Result<ReportSummary, RomTestError> {
    fs::create_dir_all(output_dir).map_err(|source| RomTestError::CreateDirectory {
        path: output_dir.to_path_buf(),
        source,
    })?;
    let screenshots_dir = output_dir.join("screenshots");
    fs::create_dir_all(&screenshots_dir).map_err(|source| RomTestError::CreateDirectory {
        path: screenshots_dir.clone(),
        source,
    })?;

    let mut html = String::new();
    let passed = outcomes.iter().filter(|outcome| outcome.passed()).count();
    let failed = outcomes.len().saturating_sub(passed);

    write!(
        html,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>\
         body{{font-family:sans-serif;margin:2rem;background:#111827;color:#e5e7eb;}}\
         h1,h2,h3,h4{{color:#f9fafb;}}\
         .category{{margin-top:2rem;padding-bottom:0.35rem;border-bottom:1px solid #374151;}}\
         table{{border-collapse:collapse;width:100%;margin:1rem 0;}}\
         th,td{{border:1px solid #374151;padding:0.5rem;vertical-align:top;}}\
         th{{background:#1f2937;text-align:left;}}\
         .pass{{color:#10b981;font-weight:700;}}\
         .fail{{color:#f87171;font-weight:700;}}\
         .case{{margin-bottom:2rem;padding:1rem;border:1px solid #374151;border-radius:0.5rem;background:#0f172a;}}\
         .thumb{{max-width:256px;height:auto;border:1px solid #374151;background:#000;}}\
         code{{white-space:nowrap;}}\
         ul{{margin:0.5rem 0 0 1.25rem;}}\
         </style></head><body>",
        escape_html(title)
    )
    .unwrap();

    write!(
        html,
        "<h1>{}</h1><p>Total cases: {} / passed: <span class=\"pass\">{}</span> / failed: <span class=\"fail\">{}</span></p>",
        escape_html(title),
        outcomes.len(),
        passed,
        failed
    )
    .unwrap();

    let mut current_category = None;
    for outcome in outcomes {
        let category = outcome.category();
        if current_category != Some(category) {
            current_category = Some(category);
            write!(
                html,
                "<h2 class=\"category\">{}</h2>",
                escape_html(category.label())
            )
            .unwrap();
        }

        match outcome {
            CaseOutcome::Completed(validation) => {
                let status_class = if validation.passed() { "pass" } else { "fail" };
                let status_label = if validation.passed() { "PASS" } else { "FAIL" };
                write!(
                    html,
                    "<section class=\"case\"><h3>{}</h3><p>{}</p>\
                     <p>Status: <span class=\"{}\">{}</span></p>\
                     <p>ROM: <code>{}</code></p>\
                     <p>Frames: {} / Steps: {} / Final screen hash: <code>0x{:016X}</code></p>",
                    escape_html(&validation.case_id),
                    escape_html(&validation.description),
                    status_class,
                    status_label,
                    escape_html(&validation.rom),
                    validation.frames,
                    validation.steps,
                    validation.final_screen_hash
                )
                .unwrap();

                write!(
                    html,
                    "<p>Audio ({} Hz): samples=<code>{}</code> hash=<code>0x{:016X}</code>",
                    validation.audio.sample_rate, validation.audio.samples, validation.audio.hash
                )
                .unwrap();
                if let Some(expected) = &validation.audio.expected {
                    write!(
                        html,
                        " expected samples=<code>{}</code> expected hash=<code>0x{:016X}</code>",
                        expected.samples, expected.hash
                    )
                    .unwrap();
                }
                html.push_str("</p>");

                if !validation.failures.is_empty() {
                    html.push_str("<h4>Failures</h4><ul>");
                    for failure in &validation.failures {
                        write!(html, "<li>{}</li>", escape_html(failure)).unwrap();
                    }
                    html.push_str("</ul>");
                }

                if !validation.screen_checks.is_empty() {
                    html.push_str(
                        "<h4>Screen checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Expected</th><th>Actual</th><th>Status</th><th>Screenshot</th>\
                         </tr></thead><tbody>",
                    );
                    for (index, check) in validation.screen_checks.iter().enumerate() {
                        let screenshot_rel = if let Some(bytes) = &check.screenshot_png {
                            let relative = format!(
                                "screenshots/{}/frame-{:06}-{:02}.png",
                                sanitize_for_path(&validation.case_id),
                                check.frame,
                                index + 1
                            );
                            let absolute = output_dir.join(&relative);
                            if let Some(parent) = absolute.parent() {
                                fs::create_dir_all(parent).map_err(|source| {
                                    RomTestError::CreateDirectory {
                                        path: parent.to_path_buf(),
                                        source,
                                    }
                                })?;
                            }
                            fs::write(&absolute, bytes).map_err(|source| {
                                RomTestError::WriteFile {
                                    path: absolute.clone(),
                                    source,
                                }
                            })?;
                            Some(relative)
                        } else {
                            None
                        };
                        let status_class = if check.passed() { "pass" } else { "fail" };
                        let status_label = if check.passed() { "PASS" } else { "FAIL" };
                        write!(
                            html,
                            "<tr><td>{}</td><td><code>0x{:016X}</code></td><td><code>0x{:016X}</code></td>\
                             <td class=\"{}\">{}</td><td>",
                            check.frame,
                            check.expected_hash,
                            check.actual_hash,
                            status_class,
                            status_label
                        )
                        .unwrap();
                        if let Some(relative) = screenshot_rel {
                            write!(
                                html,
                                "<a href=\"{}\"><img class=\"thumb\" src=\"{}\" alt=\"{} frame {}\"></a>",
                                escape_html(&relative),
                                escape_html(&relative),
                                escape_html(&validation.case_id),
                                check.frame
                            )
                            .unwrap();
                        } else {
                            html.push('—');
                        }
                        html.push_str("</td></tr>");
                    }
                    html.push_str("</tbody></table>");
                }

                html.push_str("</section>");
            }
            CaseOutcome::InternalError {
                case_id,
                description,
                rom,
                message,
                ..
            } => {
                write!(
                    html,
                    "<section class=\"case\"><h3>{}</h3><p>{}</p>\
                     <p>Status: <span class=\"fail\">ERROR</span></p>\
                     <p>ROM: <code>{}</code></p><p>{}</p></section>",
                    escape_html(case_id),
                    escape_html(description),
                    escape_html(rom),
                    escape_html(message)
                )
                .unwrap();
            }
        }
    }

    html.push_str("</body></html>");
    let report_path = output_dir.join("index.html");
    fs::write(&report_path, html).map_err(|source| RomTestError::WriteFile {
        path: report_path.clone(),
        source,
    })?;

    Ok(ReportSummary {
        report_path,
        passed,
        failed,
    })
}

fn apply_button_state(current: Buttons, button: Buttons, state: PadState) -> Buttons {
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

        match event.kind {
            RomEventKind::CheckScreen { hash } => {
                harness.on_check_screen(event.frame, hash)?;
            }
            RomEventKind::Reset => {
                harness.on_reset()?;
            }
            RomEventKind::StandardController { pad, button, state } => {
                harness.on_standard_controller(pad, button, state)?;
            }
        }

        *next_event += 1;
    }

    Ok(())
}

fn screen_hash(screen_buffer: &ScreenBuffer) -> u64 {
    let mut hasher = Crc64Hasher::new();
    screen_buffer.hash(&mut hasher);
    hasher.finish()
}

fn encode_screenshot_png(screen_buffer: &ScreenBuffer) -> Result<Vec<u8>, RomTestError> {
    let logical_size = screen_buffer.logical_size();
    let mut buffer = vec![0_u8; screen_buffer.frame_len()];
    screen_buffer.copy_display_buffer(&mut buffer);
    let mut rgba = Vec::with_capacity(buffer.len());

    for pixel in buffer.chunks_exact(4) {
        let value = u32::from_ne_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
        rgba.push((value & 0xFF) as u8);
        rgba.push(((value >> 8) & 0xFF) as u8);
        rgba.push(((value >> 16) & 0xFF) as u8);
        rgba.push(((value >> 24) & 0xFF) as u8);
    }

    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(
        &mut encoded,
        logical_size.width as u32,
        logical_size.height as u32,
    );
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    drop(writer);

    Ok(encoded.into_inner())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn sanitize_for_path(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn default_rom_root() -> PathBuf {
    PathBuf::from("../roms")
}

struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

impl Hasher for Crc64Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        self.0.clone().finalize()
    }
}

#[derive(Debug, Clone)]
pub struct HashingMixer {
    sample_rate: u32,
    samples: u64,
    checksum: u64,
}

impl HashingMixer {
    const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            samples: 0,
            checksum: Self::FNV_OFFSET_BASIS,
        }
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn checksum(&self) -> u64 {
        self.checksum
    }
}

impl MixerInput for HashingMixer {
    fn push(&mut self, data: f32) {
        self.samples += 1;
        self.checksum ^= u64::from(data.to_bits());
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

mod hex_u64 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{value:016X}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HexValueVisitor)
    }

    struct HexValueVisitor;

    impl<'de> Visitor<'de> for HexValueVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hexadecimal string like 0x0123 or an unsigned integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex(&value).map_err(E::custom)
        }
    }

    fn parse_hex(value: &str) -> Result<u64, String> {
        let trimmed = value.trim().replace('_', "");
        let digits = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"));
        if let Some(digits) = digits {
            u64::from_str_radix(digits, 16)
                .map_err(|error| format!("invalid hexadecimal value `{value}`: {error}"))
        } else {
            trimmed
                .parse::<u64>()
                .map_err(|error| format!("invalid integer value `{value}`: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_with_hex_values() {
        let mut manifest = serde_yaml::from_str::<RomManifest>(
            r#"
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: cpu/nestest.nes
    perf: true
    expected_audio:
      sample_rate: 192000
      samples: 287270
      hash: "0x34BB3FFDF962043D"
    events:
      - { frame: 15, action: check_screen, hash: "0x464033EFDAB11D8E" }
      - { frame: 15, action: standard_controller, pad: pad1, button: START, state: pressed }
"#,
        )
        .expect("manifest should parse");
        manifest.resolve_paths(&default_manifest_path());
        manifest.validate().expect("manifest should validate");
        assert!(manifest.case("cpu.nestest").unwrap().perf);
    }

    #[test]
    fn default_manifest_contains_perf_cases() {
        let manifest = load_default_manifest().expect("default manifest should load");
        assert!(manifest.case("cpu.nestest").is_some());
        assert!(manifest.case("apu.len_ctr").is_some());
        assert!(manifest.case("ppu.vbl_nmi").is_some());
    }

    #[test]
    fn resolve_manifest_paths_relative_to_manifest_file() {
        let mut manifest = serde_yaml::from_str::<RomManifest>(
            r#"
rom_root: fixtures/roms
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: cpu/nestest.nes
    events:
      - { frame: 1, action: check_screen, hash: "0x1" }
"#,
        )
        .expect("manifest should parse");

        manifest.resolve_paths(Path::new("/tmp/config/rom_tests.yaml"));

        assert_eq!(
            manifest.case("cpu.nestest").unwrap().resolved_rom_path,
            Path::new("/tmp/config")
                .join("fixtures/roms")
                .join("cpu/nestest.nes")
        );
    }

    #[test]
    fn drive_case_dispatches_frame_zero_events() {
        struct Harness {
            frame_counter: u64,
            events: Vec<String>,
        }

        impl CaseHarness for Harness {
            fn run_frame(&mut self) -> u64 {
                self.frame_counter += 1;
                1
            }

            fn frame_counter(&self) -> u64 {
                self.frame_counter
            }

            fn on_check_screen(
                &mut self,
                frame: u64,
                _expected_hash: u64,
            ) -> Result<(), RomTestError> {
                self.events.push(format!("check@{frame}"));
                Ok(())
            }

            fn on_reset(&mut self) -> Result<(), RomTestError> {
                self.events.push(format!("reset@{}", self.frame_counter));
                Ok(())
            }

            fn on_standard_controller(
                &mut self,
                _pad: ControllerPad,
                _button: ButtonCode,
                _state: PadState,
            ) -> Result<(), RomTestError> {
                self.events
                    .push(format!("controller@{}", self.frame_counter));
                Ok(())
            }
        }

        let case = RomCase {
            id: "frame-zero".to_string(),
            category: RomCategory::Cpu,
            description: "Frame-zero dispatch regression.".to_string(),
            rom: "cpu/nestest.nes".to_string(),
            perf: false,
            events: vec![
                RomEvent {
                    frame: 0,
                    kind: RomEventKind::Reset,
                },
                RomEvent {
                    frame: 0,
                    kind: RomEventKind::StandardController {
                        pad: ControllerPad::Pad1,
                        button: ButtonCode::START,
                        state: PadState::Pressed,
                    },
                },
                RomEvent {
                    frame: 1,
                    kind: RomEventKind::CheckScreen { hash: 1 },
                },
            ],
            expected_audio: None,
            resolved_rom_path: PathBuf::new(),
        };
        let mut harness = Harness {
            frame_counter: 0,
            events: Vec::new(),
        };

        let totals = drive_case(&case, &mut harness).expect("case should run");

        assert_eq!(totals.frames, 1);
        assert_eq!(totals.steps, 1);
        assert_eq!(
            harness.events,
            vec![
                "reset@0".to_string(),
                "controller@0".to_string(),
                "check@1".to_string()
            ]
        );
    }
}
