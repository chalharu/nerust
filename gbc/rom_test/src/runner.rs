use std::{path::Path, time::Instant};

use nerust_gbc_core::{
    cpu_core::Lr35902Cpu,
    memory::GbcMemoryBus,
    system::{GbcSystem, HardwareModel},
};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use super::{
    error::RomTestError,
    manifest::{CompletionSpec, CompletionStage, GbcModel, MatrixCell, MemoryCompletion},
    media,
    report::CaseResult,
    verify::{self, CheckResult},
};

/// Run every selected matrix cell and collect structured results.
pub fn run_manifest(
    rom_root: &Path,
    cells: &[MatrixCell<'_>],
    artifacts_dir: Option<&Path>,
    expected_failures: &[String],
) -> Vec<CaseResult> {
    cells
        .iter()
        .map(|cell| {
            let expected = expected_failures.iter().any(|id| id == &cell.id());
            run_case(cell, rom_root, artifacts_dir, expected)
        })
        .collect()
}

/// Run a single (case, model) matrix cell and return a structured result.
///
/// Pass criteria: every configured check passes and the run completes
/// without error. A cell with no checks passes on successful completion.
pub fn run_case(
    cell: &MatrixCell<'_>,
    rom_root: &Path,
    artifacts_dir: Option<&Path>,
    expected_failure: bool,
) -> CaseResult {
    let started = Instant::now();
    let mut acc = CaseAccumulator::default();

    let (error, error_kind) = match run_cell(cell, rom_root, artifacts_dir, &mut acc) {
        Ok(()) => (None, None),
        Err(e) => (Some(e.to_string()), Some(e.category().to_string())),
    };
    let passed = error.is_none() && acc.checks.iter().all(|check| check.passed);

    CaseResult {
        id: cell.id(),
        suite: cell.suite.name.clone(),
        model: cell.model.name().to_string(),
        description: cell.description().to_string(),
        tags: cell.tags().to_vec(),
        passed,
        expected_failure,
        checks: acc.checks,
        error,
        error_kind,
        screenshot: acc.screenshot,
        diff_image: acc.diff_image,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// Collects per-cell outputs produced during execution.
#[derive(Default)]
struct CaseAccumulator {
    checks: Vec<CheckResult>,
    screenshot: Option<String>,
    diff_image: Option<String>,
}

fn run_cell(
    cell: &MatrixCell<'_>,
    rom_root: &Path,
    artifacts_dir: Option<&Path>,
    acc: &mut CaseAccumulator,
) -> Result<(), RomTestError> {
    let (rom_path, rom_bytes) = load_rom(cell, rom_root)?;
    let GbcSystem { mut cpu, mut bus } = GbcSystem::from_rom(core_model(cell.model), rom_bytes)
        .ok_or_else(|| {
            RomTestError::InvalidManifest(format!("invalid ROM header: {}", rom_path.display()))
        })?;

    step_cycles(
        &mut bus,
        &mut cpu,
        cell.cycles(),
        cell.completion(),
        cell.verify(),
        &cell.case.inputs,
    );
    let rendered = render_frame(&bus)?;

    if let Some(dir) = artifacts_dir {
        let name = format!("{}.png", cell.id());
        save_screenshot(&rendered.png, dir, "screenshots", &name)?;
        acc.screenshot = Some(name);
    }

    verify_reference_if_present(cell, rom_root, &rendered, artifacts_dir, acc)?;
    verify_outputs(cell, &bus, &cpu, rendered.crc, acc)?;

    Ok(())
}

fn load_rom(
    cell: &MatrixCell<'_>,
    rom_root: &Path,
) -> Result<(std::path::PathBuf, Vec<u8>), RomTestError> {
    let rom_path = cell.rom_path(rom_root);
    if !rom_path.exists() {
        return Err(RomTestError::InvalidManifest(format!(
            "ROM not found: {}",
            rom_path.display()
        )));
    }
    let rom_bytes = std::fs::read(&rom_path)?;
    Ok((rom_path, rom_bytes))
}

fn core_model(model: GbcModel) -> HardwareModel {
    match model {
        GbcModel::Dmg0 => HardwareModel::Dmg0,
        GbcModel::Dmg => HardwareModel::Dmg,
        GbcModel::CgbC => HardwareModel::CgbC,
        GbcModel::CgbD => HardwareModel::CgbD,
        GbcModel::Agb => HardwareModel::Agb,
    }
}

fn verify_reference_if_present(
    cell: &MatrixCell<'_>,
    rom_root: &Path,
    rendered: &RenderedFrame,
    artifacts_dir: Option<&Path>,
    acc: &mut CaseAccumulator,
) -> Result<(), RomTestError> {
    let Some(ref_path) = cell.reference_path(rom_root) else {
        return Ok(());
    };
    if !ref_path.exists() {
        return Err(RomTestError::InvalidManifest(format!(
            "reference image not found: {}",
            ref_path.display()
        )));
    }
    let ref_png = std::fs::read(&ref_path)?;
    let diff_png = verify::verify_reference(
        &verify::FramePixels {
            rgba: &rendered.rgba,
            width: rendered.width as u32,
            height: rendered.height as u32,
        },
        &ref_png,
        &ref_path.display().to_string(),
        &mut acc.checks,
    )?;
    if let (Some(png), Some(dir)) = (diff_png, artifacts_dir) {
        let name = format!("{}_diff.png", cell.id());
        save_screenshot(&png, dir, "diffs", &name)?;
        acc.diff_image = Some(name);
    }
    Ok(())
}

fn verify_outputs(
    cell: &MatrixCell<'_>,
    bus: &GbcMemoryBus,
    cpu: &Lr35902Cpu,
    crc: u32,
    acc: &mut CaseAccumulator,
) -> Result<(), RomTestError> {
    let serial_output = bus.serial_output();
    cell.verify()
        .verify_serial(serial_output, &mut acc.checks)?;
    cell.verify().verify_frame(crc, &mut acc.checks);
    cell.verify().verify_memory(bus, &mut acc.checks)?;
    if cell.suite.name == "aappleby_gbmicrotest" && bus.read(0xFF82) != 0x01 {
        let actual = bus.read(0xFF80);
        let expected = bus.read(0xFF81);
        acc.checks.push(CheckResult {
            name: "gbmicro-result@$FF80/$FF81".to_string(),
            expected: format!("${expected:02X}"),
            actual: format!("${actual:02X}"),
            passed: actual == expected,
        });
    }
    cell.verify()
        .verify_registers(cpu.registers(), &mut acc.checks);
    Ok(())
}

struct RenderedFrame {
    png: Vec<u8>,
    /// Stride-aware RGBA pixels (160×144×4).
    rgba: Vec<u8>,
    /// CRC32 of `rgba`.
    crc: u32,
    width: usize,
    height: usize,
}

fn render_frame(bus: &GbcMemoryBus) -> Result<RenderedFrame, RomTestError> {
    let mut fb = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
    fb.resize(160, 144);
    bus.render_frame(&mut fb);
    let png = media::encode_screenshot_png(&fb)?;

    let stride = fb.stride();
    let w = fb.width();
    let h = fb.height();
    let src = fb.as_ref();
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row_start = y * stride;
        rgba.extend_from_slice(&src[row_start..row_start + w * 4]);
    }
    let crc = verify::crc32(&rgba);
    Ok(RenderedFrame {
        png,
        rgba,
        crc,
        width: w,
        height: h,
    })
}

fn step_cycles(
    bus: &mut GbcMemoryBus,
    cpu: &mut Lr35902Cpu,
    cycles: usize,
    completion: Option<&CompletionSpec>,
    verify: &crate::verify::VerifySpec,
    inputs: &[crate::manifest::InputEvent],
) {
    let mut tracker = CompletionTracker::default();
    let mut next_input = 0;
    for cycle in 0..cycles {
        if inputs
            .get(next_input)
            .is_some_and(|event| event.cycle == cycle)
        {
            let buttons = inputs[next_input]
                .buttons
                .iter()
                .fold(0xFF, |state, button| state & !button.mask());
            bus.set_joypad(buttons);
            next_input += 1;
        }
        // T-cycle synchronized: CPU + PPU advance at 1 T-cycle per call.
        // 4 calls = 1 M-cycle for CPU, 4 T-cycles for PPU.
        for _ in 0..4 {
            bus.step_tcycle(cpu);
        }
        if let Some(spec) = completion
            && cycle.is_multiple_of(spec.poll_interval)
        {
            let stage = &spec.stages[tracker.stage];
            if tracker.observe(stage_matches(stage, bus, cpu, verify), spec.stages.len()) {
                return;
            }
        }
    }
}

fn stage_matches(
    stage: &CompletionStage,
    bus: &GbcMemoryBus,
    cpu: &Lr35902Cpu,
    verify: &crate::verify::VerifySpec,
) -> bool {
    stage
        .memory
        .iter()
        .all(|condition| memory_matches(condition, bus))
        && (!stage.serial_hash || verify.serial_hash_matches(bus.serial_output()))
        && stage.registers.matches(cpu.registers())
}

fn memory_matches(condition: &MemoryCompletion, bus: &GbcMemoryBus) -> bool {
    let Ok(address) = crate::verify::parse_hex(&condition.address).and_then(|value| {
        u16::try_from(value).map_err(|_| {
            RomTestError::InvalidManifest("completion address out of range".to_string())
        })
    }) else {
        return false;
    };
    let actual = bus.debug_read(address);
    if let Some(value) = &condition.value {
        return crate::verify::parse_hex(value).is_ok_and(|value| actual as u64 == value);
    }
    condition.not_value.as_ref().is_some_and(|value| {
        crate::verify::parse_hex(value).is_ok_and(|value| actual as u64 != value)
    })
}

#[derive(Default)]
struct CompletionTracker {
    stage: usize,
}

impl CompletionTracker {
    fn observe(&mut self, matches: bool, stage_count: usize) -> bool {
        if matches {
            self.stage += 1;
        }
        self.stage == stage_count
    }
}

fn save_screenshot(
    png_data: &[u8],
    root: &Path,
    subdir: &str,
    name: &str,
) -> Result<(), RomTestError> {
    let dir = root.join(subdir);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(name), png_data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CompletionTracker;

    #[test]
    fn completion_tracker_requires_each_stage_in_order() {
        let mut completion = CompletionTracker::default();
        assert!(!completion.observe(false, 2));
        assert!(!completion.observe(true, 2));
        assert!(completion.observe(true, 2));
    }

    #[test]
    fn completion_tracker_waits_while_stage_does_not_match() {
        let mut completion = CompletionTracker::default();
        assert!(!completion.observe(false, 1));
        assert!(completion.observe(true, 1));
    }
}
