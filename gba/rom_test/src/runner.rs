use std::{path::Path, time::Instant};

use nerust_gba_core::{memory::GbaMemoryBus, system::GbaSystem};

use crate::{
    error::RomTestError,
    manifest::{CompletionStage, MemoryCompletion, SelectedCase},
    media,
    report::CaseResult,
};

pub fn run_manifest(
    rom_root: &Path,
    cases: &[SelectedCase<'_>],
    artifacts_dir: Option<&Path>,
    expected_failures: &[String],
) -> Vec<CaseResult> {
    cases
        .iter()
        .map(|case| {
            let expected = expected_failures.iter().any(|id| id == &case.case.id);
            run_case(case, rom_root, artifacts_dir, expected)
        })
        .collect()
}

pub fn run_case(
    selected: &SelectedCase<'_>,
    rom_root: &Path,
    artifacts_dir: Option<&Path>,
    expected_failure: bool,
) -> CaseResult {
    let started = Instant::now();
    let mut executed_tcycles = 0;
    let mut completed_early = false;
    let mut acc = CaseAccumulator::default();

    let (error, error_kind) = match run_case_inner(
        selected,
        rom_root,
        &mut executed_tcycles,
        &mut completed_early,
        artifacts_dir,
        &mut acc,
    ) {
        Ok(()) => (None, None),
        Err(e) => (Some(e.to_string()), Some(e.category().to_string())),
    };

    let checks = std::mem::take(&mut acc.checks);
    let passed = error.is_none() && checks.iter().all(|check| check.passed);
    CaseResult {
        id: selected.case.id.clone(),
        suite: selected.suite.name.clone(),
        description: selected.case.description.clone(),
        passed,
        expected_failure,
        checks,
        error,
        error_kind,
        screenshot: acc.screenshot,
        diff_image: acc.diff_image,
        executed_tcycles,
        completed_early,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

#[derive(Default)]
struct CaseAccumulator {
    checks: Vec<crate::verify::CheckResult>,
    screenshot: Option<String>,
    diff_image: Option<String>,
}

fn run_case_inner(
    selected: &SelectedCase<'_>,
    rom_root: &Path,
    executed_tcycles: &mut usize,
    completed_early: &mut bool,
    artifacts_dir: Option<&Path>,
    acc: &mut CaseAccumulator,
) -> Result<(), RomTestError> {
    let rom_path = rom_root.join(&selected.suite.name).join(&selected.case.rom);
    if !rom_path.is_file() {
        return Err(RomTestError::InvalidManifest(format!(
            "ROM not found: {}",
            rom_path.display()
        )));
    }
    let rom = std::fs::read(&rom_path)?;
    let mut system = GbaSystem::from_test_rom(rom)
        .ok_or_else(|| RomTestError::InvalidRom(rom_path.display().to_string()))?;

    // For interactive ROMs like armwrestler the initial TESTNUM is written by
    // the boot code (mov r0,#10 @ MENU). Apply setup after that store has
    // executed so it is not overwritten. 1000 T-cycles is enough for the
    // 0x080000C0 init sequence to complete.
    if !selected.case.setup.is_empty() {
        for _ in 0..1000 {
            system.step_tcycle();
        }
        for entry in &selected.case.setup {
            let addr = crate::verify::parse_hex(&entry.address)? as u32;
            let val = crate::verify::parse_hex(&entry.value)? as u32;
            match entry.width {
                1 => system.bus.write8(addr, val as u8),
                2 => system.bus.write16(addr, val as u16),
                4 => system.bus.write32(addr, val),
                _ => {}
            }
        }
    }

    let mut completion_tracker = CompletionTracker::default();
    for cycle in 0..selected.case.cycles {
        system.step_tcycle();
        *executed_tcycles = cycle + 1;
        if let Some(completion) = selected.completion
            && cycle.is_multiple_of(completion.poll_interval)
            && completion_tracker.observe(
                stage_matches(&completion.stages[completion_tracker.stage], &mut system),
                completion.stages.len(),
            )
        {
            *completed_early = true;
            break;
        }
    }

    // Capture screenshot
    let rendered = render_frame(&system)?;
    if let Some(dir) = artifacts_dir {
        let name = format!("{}.png", selected.case.id);
        save_screenshot(&rendered.png, dir, "screenshots", &name)?;
        acc.screenshot = Some(name);
    }

    // Verify reference if present (check for .png next to rom)
    verify_reference_if_present(selected, rom_root, &rendered, artifacts_dir, acc)?;

    // Verify memory/registers/frame_pixels
    let mut checks = selected
        .case
        .verify
        .verify(&mut system.bus, system.cpu.registers())?;
    // Also verify frame_pixels already includes frame_buffer check, but we also want to verify full frame if needed
    acc.checks.append(&mut checks);
    Ok(())
}

fn stage_matches(stage: &CompletionStage, system: &mut GbaSystem) -> bool {
    stage
        .memory
        .iter()
        .all(|condition| memory_matches(condition, &mut system.bus))
        && stage.registers.matches(system.cpu.registers())
}

fn memory_matches(condition: &MemoryCompletion, bus: &mut GbaMemoryBus) -> bool {
    let Ok(address) = crate::verify::parse_hex(&condition.address).map(|value| value as u32) else {
        return false;
    };
    let actual = match condition.width {
        1 => u32::from(bus.read8(address)),
        2 => u32::from(bus.read16(address)),
        4 => bus.read32(address),
        _ => return false,
    };
    if let Some(value) = &condition.value {
        return crate::verify::parse_hex(value).is_ok_and(|value| u64::from(actual) == value);
    }
    condition.not_value.as_ref().is_some_and(|value| {
        crate::verify::parse_hex(value).is_ok_and(|value| u64::from(actual) != value)
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

struct RenderedFrame {
    png: Vec<u8>,
    rgba: Vec<u8>,
    width: usize,
    height: usize,
}

fn render_frame(system: &GbaSystem) -> Result<RenderedFrame, RomTestError> {
    use nerust_gba_core::ppu::{HEIGHT, WIDTH};

    let fb = system.frame_buffer();
    // fb is &[u32] where each u32 is 0xRRGGBBAA in little-endian (rgba8888)
    // Convert to RGBA bytes
    let mut rgba = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for &pixel in fb {
        rgba.extend_from_slice(&pixel.to_le_bytes());
    }

    let png = media::encode_rgba_png(WIDTH as u32, HEIGHT as u32, &rgba)?;

    Ok(RenderedFrame {
        png,
        rgba,
        width: WIDTH,
        height: HEIGHT,
    })
}

fn verify_reference_if_present(
    selected: &SelectedCase<'_>,
    rom_root: &Path,
    rendered: &RenderedFrame,
    artifacts_dir: Option<&Path>,
    acc: &mut CaseAccumulator,
) -> Result<(), RomTestError> {
    // Look for reference PNG next to ROM: same path but .png instead of .gba
    let rom_path = rom_root.join(&selected.suite.name).join(&selected.case.rom);
    let ref_path = rom_path.with_extension("png");
    // Also try expected.png / expected.jpg in same dir as ROM (for nba-emu)
    let alt_png = rom_path.parent().map(|d| d.join("expected.png")).unwrap_or_default();
    let alt_jpg = rom_path.parent().map(|d| d.join("expected.jpg")).unwrap_or_default();
    let ref_path = if ref_path.exists() {
        Some(ref_path)
    } else if alt_png.exists() {
        Some(alt_png)
    } else if alt_jpg.exists() {
        Some(alt_jpg)
    } else {
        None
    };

    let Some(ref_path) = ref_path else {
        return Ok(());
    };

    let ref_png = std::fs::read(&ref_path)?;
    let mut checks = Vec::new();
    let diff_png = crate::verify::verify_reference(
        &crate::verify::FramePixels {
            rgba: &rendered.rgba,
            width: rendered.width as u32,
            height: rendered.height as u32,
        },
        &ref_png,
        &ref_path.display().to_string(),
        &mut checks,
    )?;
    acc.checks.extend(checks);
    if let (Some(png), Some(dir)) = (diff_png, artifacts_dir) {
        let name = format!("{}_diff.png", selected.case.id);
        save_screenshot(&png, dir, "diffs", &name)?;
        acc.diff_image = Some(name);
    }
    Ok(())
}

fn save_screenshot(
    png_data: &[u8],
    root: &Path,
    subdir: &str,
    name: &str,
) -> Result<(), RomTestError> {
    let dir = root.join(subdir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        RomTestError::InvalidManifest(format!("failed to create {} dir: {e}", dir.display()))
    })?;
    std::fs::write(dir.join(name), png_data)
        .map_err(|e| RomTestError::InvalidManifest(format!("failed to write screenshot: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        manifest::RomCase,
        verify::{MemoryEntry, VerifySpec},
    };
    use nerust_gba_core::cartridge::header::finalize_test_gba_rom;

    #[test]
    fn completion_tracker_requires_ordered_matches() {
        let mut tracker = CompletionTracker::default();
        assert!(!tracker.observe(false, 2));
        assert!(!tracker.observe(true, 2));
        assert!(tracker.observe(true, 2));
    }

    #[test]
    fn executes_rom_and_verifies_memory() {
        let root = std::env::temp_dir().join(format!("nerust-gba-rom-test-{}", std::process::id()));
        let suite_dir = root.join("synthetic");
        std::fs::create_dir_all(&suite_dir).unwrap();
        let mut rom = vec![0u8; 0x200];
        rom[0..4].copy_from_slice(&0xEA00_002Eu32.to_le_bytes()); // B 0x080000C0
        rom[0xC0..0xC4].copy_from_slice(&0xE3A0_0001u32.to_le_bytes()); // MOV R0,#1
        rom[0xC4..0xC8].copy_from_slice(&0xE3A0_1402u32.to_le_bytes()); // MOV R1,#0x02000000
        rom[0xC8..0xCC].copy_from_slice(&0xE581_0000u32.to_le_bytes()); // STR R0,[R1]
        rom[0xCC..0xD0].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes()); // B .
        finalize_test_gba_rom(&mut rom);
        std::fs::write(suite_dir.join("pass.gba"), rom).unwrap();

        let suite = crate::manifest::RomSuite {
            name: "synthetic".into(),
            cases: Vec::new(),
            case_patterns: Vec::new(),
        };
        let case = RomCase {
            id: "synthetic_pass".into(),
            rom: "pass.gba".into(),
            cycles: 200,
            completion: None,
            description: "synthetic ARM program".into(),
            verify: VerifySpec {
                memory: vec![MemoryEntry {
                    address: "0x02000000".into(),
                    value: "1".into(),
                    width: 1,
                }],
                ..Default::default()
            },
        };
        let selected = SelectedCase {
            suite: &suite,
            case: &case,
            completion: None,
        };
        let result = run_case(&selected, &root, None, false);
        let _ = std::fs::remove_dir_all(&root);
        assert!(result.passed, "{:?}", result.error);
        assert_eq!(result.checks.len(), 1);
        assert!(result.checks[0].passed);
    }
}
