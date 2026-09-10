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
    let allowed: std::collections::HashSet<&str> = selected
        .case
        .expected_checks
        .iter()
        .map(String::as_str)
        .collect();
    let passed = error.is_none()
        && checks
            .iter()
            .all(|check| check.passed || allowed.contains(check.name.as_str()));
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
    logs: Vec<nerust_gba_core::memory::MgbaDebugLog>,
}

/// T-cycles per video frame (228 lines x 308 dots x 4).
const CYCLES_PER_FRAME: usize = 280_896;
/// Released-key settle between script steps, in frames.
const SCRIPT_SETTLE_FRAMES: usize = 2;

fn drain_logs(system: &mut GbaSystem, acc: &mut CaseAccumulator) {
    acc.logs.extend(system.bus.drain_mgba_debug_logs());
}

fn run_fixed_cycles(
    selected: &SelectedCase<'_>,
    system: &mut GbaSystem,
    executed_tcycles: &mut usize,
    completed_early: &mut bool,
) -> Result<(), RomTestError> {
    let mut next_input = 0;
    let mut completion_tracker = CompletionTracker::default();
    for cycle in 0..selected.case.cycles {
        // Apply input at this cycle
        if next_input < selected.case.inputs.len()
            && selected.case.inputs[next_input].cycle == cycle
        {
            let keyinput = selected.case.inputs[next_input]
                .buttons
                .iter()
                .fold(0x03FFu16, |state, btn| state & !btn.mask());
            system.bus.set_keyinput(keyinput);
            next_input += 1;
        }

        system.step_tcycle();
        *executed_tcycles = cycle + 1;
        if let Some(completion) = selected.completion
            && cycle.is_multiple_of(completion.poll_interval)
            && completion_tracker.observe(
                stage_matches(&completion.stages[completion_tracker.stage], system),
                completion.stages.len(),
            )
        {
            *completed_early = true;
            break;
        }
    }
    Ok(())
}

/// Run a headless input script: hold each step's buttons until its fresh
/// log marker arrives (or its frame budget elapses), release, settle.
fn run_script(
    selected: &SelectedCase<'_>,
    system: &mut GbaSystem,
    executed_tcycles: &mut usize,
    logs: &mut Vec<nerust_gba_core::memory::MgbaDebugLog>,
) -> Result<(), RomTestError> {
    let mut cursor = 0usize;
    for (index, step) in selected.case.script.iter().enumerate() {
        let keyinput = step
            .press
            .iter()
            .fold(0x03FFu16, |state, btn| state & !btn.mask());
        system.bus.set_keyinput(keyinput);
        if let Some(marker) = &step.until_log {
            loop {
                if *executed_tcycles >= selected.case.cycles {
                    let texts: Vec<&str> = logs.iter().map(|log| log.text.as_str()).collect();
                    return Err(RomTestError::Timeout(format!(
                        "case `{}` script step {index} never logged `{marker}` (saw {texts:?})",
                        selected.case.id
                    )));
                }
                system.step_tcycle();
                *executed_tcycles += 1;
                logs.extend(system.bus.drain_mgba_debug_logs());
                if logs[cursor..].iter().any(|log| log.text.contains(marker)) {
                    break;
                }
            }
        } else if let Some(frames) = step.wait_frames {
            let target = executed_tcycles.saturating_add(
                usize::try_from(frames)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(CYCLES_PER_FRAME),
            );
            while *executed_tcycles < target && *executed_tcycles < selected.case.cycles {
                system.step_tcycle();
                *executed_tcycles += 1;
                logs.extend(system.bus.drain_mgba_debug_logs());
            }
        }
        // Release so the guest observes key transitions, then settle.
        system.bus.set_keyinput(0x03FF);
        for _ in 0..SCRIPT_SETTLE_FRAMES.saturating_mul(CYCLES_PER_FRAME) {
            if *executed_tcycles >= selected.case.cycles {
                break;
            }
            system.step_tcycle();
            *executed_tcycles += 1;
            logs.extend(system.bus.drain_mgba_debug_logs());
        }
        cursor = logs.len();
    }
    Ok(())
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
    drain_logs(&mut system, acc);

    if selected.case.script.is_empty() {
        run_fixed_cycles(selected, &mut system, executed_tcycles, completed_early)?;
    } else {
        run_script(selected, &mut system, executed_tcycles, &mut acc.logs)?;
    }
    drain_logs(&mut system, acc);

    // Capture screenshot — render what's currently in VRAM
    let rendered = render_frame(&system)?;
    if let Some(dir) = artifacts_dir {
        let name = format!("{}.png", selected.case.id);
        if selected.case.skip_screenshot {
            let _ = std::fs::remove_file(dir.join("screenshots").join(&name));
        } else {
            save_screenshot(&rendered.png, dir, "screenshots", &name)?;
            acc.screenshot = Some(name);
        }
    }

    // Verify reference if present (check for .png next to rom)
    // NOTE: `skip_screenshot` only suppresses saving the artifact screenshot
    // above; the reference compare below still runs when a sibling .png /
    // expected.png(.jpg) exists. That compare is load-bearing: cases with no
    // `verify:` block (e.g. the six BIOSSound* driver cases) rely on it as
    // their only check, since an empty check list is a hard failure below.
    verify_reference_if_present(selected, rom_root, &rendered, artifacts_dir, acc)?;

    // Verify memory/registers/frame_pixels
    let mut checks = selected
        .case
        .verify
        .verify(&mut system.bus, system.cpu.registers())?;
    // Also verify frame_pixels already includes frame_buffer check, but we also want to verify full frame if needed
    acc.checks.append(&mut checks);
    // Branch one ROM's debug log into per-subtest checks (mgba-emu/suite).
    if let Some(suite_log) = &selected.case.verify.suite_log {
        acc.checks
            .extend(crate::verify::verify_suite_log(&acc.logs, suite_log));
        // Enrich failures with `Got X vs Y` details from the SRAM log.
        let mut sram = Vec::with_capacity(0x8000);
        for addr in 0x0E00_0000..0x0E00_8000 {
            sram.push(system.bus.read8(addr));
        }
        let text = String::from_utf8_lossy(&sram).into_owned();
        crate::verify::enrich_suite_log_checks(&mut acc.checks, &text);
    }
    if acc.checks.is_empty() {
        acc.checks.push(crate::verify::CheckResult {
            name: "verification".into(),
            expected: "at least one verification check".into(),
            actual: "none configured".into(),
            passed: false,
        });
    }
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
    let suite_dir = rom_root.join(&selected.suite.name);

    // Check for per-case reference first (highest priority)
    let ref_path = if let Some(ref_ref) = &selected.case.reference {
        let case_ref = suite_dir.join(ref_ref);
        if case_ref.exists() {
            Some(case_ref)
        } else {
            None
        }
    } else {
        // Fall back to ROM-based resolution
        let rom_path = suite_dir.join(&selected.case.rom);
        let ref_path = rom_path.with_extension("png");
        // Also try expected.png / expected.jpg in same dir as ROM (for nba-emu)
        let alt_png = rom_path
            .parent()
            .map(|d| d.join("expected.png"))
            .unwrap_or_default();
        let alt_jpg = rom_path
            .parent()
            .map(|d| d.join("expected.jpg"))
            .unwrap_or_default();
        if ref_path.exists() {
            Some(ref_path)
        } else if alt_png.exists() {
            Some(alt_png)
        } else if alt_jpg.exists() {
            Some(alt_jpg)
        } else {
            None
        }
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
    if checks.is_empty() {
        checks.push(crate::verify::CheckResult {
            name: "reference image".into(),
            expected: ref_path.display().to_string(),
            actual: "matched".into(),
            passed: true,
        });
    }
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
            inputs: Vec::new(),
            reference: None,
            skip_screenshot: false,
            script: Vec::new(),
            expected_checks: Vec::new(),
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

    #[test]
    fn scripted_case_branches_suite_log() {
        use crate::manifest::ScriptStep;
        use crate::verify::SuiteLogVerify;

        // Hand-assembled ROM: mgba_open handshake, then four debug-log
        // lines ("BEGIN: syn", "PASS: syn ok", "FAIL: syn bad",
        // "END: 1/2"), then spin. LDR-literal pools are laid out after
        // the code and offsets patched in.
        let code_base = 0x080000C0u32;
        let mut code: Vec<u32> = Vec::new();
        let mut pool: Vec<u32> = Vec::new();
        // (code index to patch, pool index)
        let mut fixups: Vec<(usize, usize)> = Vec::new();
        let ldr = |code: &mut Vec<u32>,
                   pool: &mut Vec<u32>,
                   fixups: &mut Vec<(usize, usize)>,
                   rd: u32,
                   val: u32| {
            pool.push(val);
            fixups.push((code.len(), pool.len() - 1));
            code.push(0xE59F_0000 | (rd << 12));
        };
        ldr(&mut code, &mut pool, &mut fixups, 1, 0x04FFF780);
        ldr(&mut code, &mut pool, &mut fixups, 0, 0xC0DE);
        code.push(0xE1C1_00B0); // STRH R0,[R1]
        // Pool entry for the string-buffer base, reloaded into R1 before
        // every line (mgba_printf always formats from the buffer start).
        pool.push(0x04FFF600);
        let buf_pool = pool.len() - 1;
        let ldr_r1 = |code: &mut Vec<u32>, fixups: &mut Vec<(usize, usize)>| {
            fixups.push((code.len(), buf_pool));
            code.push(0xE59F_1000);
        };
        ldr_r1(&mut code, &mut fixups);
        ldr(&mut code, &mut pool, &mut fixups, 2, 0x04FFF700);
        ldr(&mut code, &mut pool, &mut fixups, 3, 0x104);
        let lines: [&[u32]; 4] = [
            &[0x4947_4542, 0x7320_3A4E, 0x0000_6E79], // "BEGIN: syn"
            &[0x5353_4150, 0x7973_203A, 0x6B6F_206E], // "PASS: syn ok"
            &[0x4C49_4146, 0x7973_203A, 0x6162_206E, 0x0000_0064], // "FAIL: syn bad"
            &[0x3A44_4E45, 0x322F_3120],              // "END: 1/2"
        ];
        for words in lines {
            ldr_r1(&mut code, &mut fixups);
            for &w in words {
                ldr(&mut code, &mut pool, &mut fixups, 0, w);
                code.push(0xE481_0004); // STR R0,[R1],#4
            }
            code.push(0xE1C2_30B0); // STRH R3,[R2]
        }
        code.push(0xEAFF_FFFE); // B .
        let pool_base = code_base + (code.len() as u32) * 4;
        for (code_idx, pool_idx) in &fixups {
            let instr_addr = code_base + (*code_idx as u32) * 4;
            let target = pool_base + (*pool_idx as u32) * 4;
            let off = target.wrapping_sub(instr_addr + 8);
            assert!(off < 0x1000, "literal pool out of range");
            code[*code_idx] |= off;
        }

        let mut rom = vec![0u8; 0x400];
        rom[0..4].copy_from_slice(&0xEA00_002Eu32.to_le_bytes()); // B 0x080000C0
        for (i, &w) in code.iter().enumerate() {
            rom[0xC0 + i * 4..0xC0 + (i + 1) * 4].copy_from_slice(&w.to_le_bytes());
        }
        let pool_off = 0xC0 + code.len() * 4;
        for (i, &w) in pool.iter().enumerate() {
            rom[pool_off + i * 4..pool_off + (i + 1) * 4].copy_from_slice(&w.to_le_bytes());
        }
        finalize_test_gba_rom(&mut rom);

        let root =
            std::env::temp_dir().join(format!("nerust-gba-suite-log-{}", std::process::id()));
        let suite_dir = root.join("synthetic");
        std::fs::create_dir_all(&suite_dir).unwrap();
        std::fs::write(suite_dir.join("suite.gba"), rom).unwrap();

        let suite = crate::manifest::RomSuite {
            name: "synthetic".into(),
            cases: Vec::new(),
            case_patterns: Vec::new(),
        };
        let case = RomCase {
            id: "synthetic_suite".into(),
            rom: "suite.gba".into(),
            cycles: 100_000,
            completion: None,
            description: "synthetic debug-log ROM".into(),
            verify: VerifySpec {
                suite_log: Some(SuiteLogVerify {
                    begin: "BEGIN: syn".into(),
                    end: "END:".into(),
                }),
                ..Default::default()
            },
            inputs: Vec::new(),
            reference: None,
            skip_screenshot: true,
            script: vec![ScriptStep {
                press: Vec::new(),
                until_log: Some("END:".into()),
                wait_frames: None,
            }],
            expected_checks: vec!["syn bad".into()],
        };
        let selected = SelectedCase {
            suite: &suite,
            case: &case,
            completion: None,
        };
        let result = run_case(&selected, &root, None, false);
        let _ = std::fs::remove_dir_all(&root);
        assert!(result.passed, "{:?} {:?}", result.error, result.checks);
        assert_eq!(result.checks.len(), 2);
        assert!(result.checks.iter().any(|c| c.name == "syn ok" && c.passed));
        assert!(
            result
                .checks
                .iter()
                .any(|c| c.name == "syn bad" && !c.passed)
        );
    }

    #[test]
    fn rejects_cases_without_verification() {
        let root = std::env::temp_dir().join(format!("nerust-gba-rom-test-{}", std::process::id()));
        let suite_dir = root.join("synthetic");
        std::fs::create_dir_all(&suite_dir).unwrap();
        let mut rom = vec![0u8; 0x200];
        rom[0..4].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes());
        finalize_test_gba_rom(&mut rom);
        std::fs::write(suite_dir.join("unchecked.gba"), rom).unwrap();
        let screenshots = root.join("screenshots");
        std::fs::create_dir_all(&screenshots).unwrap();
        let stale_screenshot = screenshots.join("synthetic_unchecked.png");
        std::fs::write(&stale_screenshot, b"stale").unwrap();

        let suite = crate::manifest::RomSuite {
            name: "synthetic".into(),
            cases: Vec::new(),
            case_patterns: Vec::new(),
        };
        let case = RomCase {
            id: "synthetic_unchecked".into(),
            rom: "unchecked.gba".into(),
            cycles: 4,
            completion: None,
            description: "synthetic ARM program".into(),
            verify: VerifySpec::default(),
            inputs: Vec::new(),
            reference: None,
            skip_screenshot: true,
            script: Vec::new(),
            expected_checks: Vec::new(),
        };
        let selected = SelectedCase {
            suite: &suite,
            case: &case,
            completion: None,
        };
        let result = run_case(&selected, &root, Some(&root), false);
        assert!(!result.passed);
        assert_eq!(result.checks[0].name, "verification");
        assert!(result.screenshot.is_none());
        assert!(!stale_screenshot.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
