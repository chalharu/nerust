use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use nerust_gbc_core::{
    cartridge::Cartridge, cartridge_header::CartridgeHeader, cpu_core::Lr35902Cpu,
    memory::GbcMemoryBus,
};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use super::{
    error::RomTestError,
    manifest::{GbcModel, MatrixCell},
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
    let (_rom_path, rom_bytes, header) = load_rom(cell, rom_root)?;
    let font_bank1 = extract_font_bank1(&rom_bytes);
    let mut bus = setup_bus(cell, &header, rom_bytes, &font_bank1);
    let mut cpu = setup_cpu(cell, &header);

    step_cycles(&mut bus, &mut cpu, cell.cycles());
    dump_text_if_requested(&bus);
    let rendered = render_frame(&bus)?;

    if let Some(dir) = artifacts_dir {
        let name = format!("{}.png", cell.id());
        save_screenshot(&rendered.png, dir, "screenshots", &name)?;
        acc.screenshot = Some(name);
    }

    verify_reference_if_present(cell, rom_root, &rendered, artifacts_dir, acc)?;
    verify_outputs(cell, &bus, rendered.crc, acc)?;

    Ok(())
}

fn load_rom(
    cell: &MatrixCell<'_>,
    rom_root: &Path,
) -> Result<(PathBuf, Vec<u8>, CartridgeHeader), RomTestError> {
    let rom_path = cell.rom_path(rom_root);
    if !rom_path.exists() {
        return Err(RomTestError::InvalidManifest(format!(
            "ROM not found: {}",
            rom_path.display()
        )));
    }
    let rom_bytes = std::fs::read(&rom_path)?;
    let header = CartridgeHeader::parse(&rom_bytes).ok_or_else(|| {
        RomTestError::InvalidManifest(format!("invalid ROM header: {}", rom_path.display()))
    })?;
    Ok((rom_path, rom_bytes, header))
}

fn extract_font_bank1(rom_bytes: &[u8]) -> Vec<u8> {
    if rom_bytes.len() > 0x4000 {
        rom_bytes[0x4000..rom_bytes.len().min(0x4800)].to_vec()
    } else {
        Vec::new()
    }
}

fn setup_bus(
    cell: &MatrixCell<'_>,
    header: &CartridgeHeader,
    rom_bytes: Vec<u8>,
    font_bank1: &[u8],
) -> GbcMemoryBus {
    let mbc = nerust_gbc_core::cartridge_mbc::create_mbc(header, rom_bytes, None);
    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cartridge(Cartridge::new(mbc));
    if !font_bank1.is_empty() {
        bus.load_font_tiles(font_bank1);
    }

    let hw_is_cgb = matches!(cell.model, GbcModel::CgbC | GbcModel::CgbD | GbcModel::Agb);
    let rom_is_cgb = header.cgb_flag & 0x80 != 0;
    bus.set_cgb_mode(hw_is_cgb);
    bus.set_cgb_revision_d(matches!(cell.model, GbcModel::CgbD | GbcModel::Agb));
    bus.set_cgb_game(hw_is_cgb && rom_is_cgb);
    match cell.model {
        GbcModel::Dmg0 => bus.set_boot_counter(0x182F),
        GbcModel::Dmg => bus.set_boot_counter(0xABCB),
        GbcModel::CgbC | GbcModel::CgbD | GbcModel::Agb => bus.set_boot_counter(0x2677),
    }
    if matches!(cell.model, GbcModel::Dmg0) {
        bus.set_ppu_frame_phase(66220);
    }
    bus.set_post_boot_io(hw_is_cgb);
    bus.set_post_boot_key1(hw_is_cgb && rom_is_cgb);
    bus
}

fn setup_cpu(cell: &MatrixCell<'_>, header: &CartridgeHeader) -> Lr35902Cpu {
    let hw_is_cgb = matches!(cell.model, GbcModel::CgbC | GbcModel::CgbD | GbcModel::Agb);
    let rom_is_cgb = header.cgb_flag & 0x80 != 0;
    let mut cpu = match cell.model {
        GbcModel::Dmg0 => Lr35902Cpu::with_model(nerust_gbc_core::cpu_core::GbcModel::Dmg0),
        GbcModel::Dmg => Lr35902Cpu::with_model(nerust_gbc_core::cpu_core::GbcModel::Dmg),
        GbcModel::CgbC | GbcModel::CgbD => {
            Lr35902Cpu::with_model(nerust_gbc_core::cpu_core::GbcModel::Cgb)
        }
        GbcModel::Agb => Lr35902Cpu::with_model(nerust_gbc_core::cpu_core::GbcModel::Agb),
    };
    if hw_is_cgb && !rom_is_cgb {
        cpu.set_cgb_dmg_mode_registers();
    }
    cpu.registers_mut().set_pc(0x0100);
    cpu
}

fn dump_text_if_requested(bus: &GbcMemoryBus) {
    if std::env::var("DUMP_TEXT").is_err() {
        return;
    }
    let toa = bus.read(0xD883) as u16 | ((bus.read(0xD884) as u16) << 8);
    let mut text = Vec::new();
    for a in 0xA004u16..0xC000 {
        let b = bus.read(a);
        if b == 0 {
            break;
        }
        text.push(b);
    }
    eprintln!(
        "TEXT_DUMP a000={:02x} toa={:04x} len={} str={}",
        bus.read(0xA000),
        toa,
        text.len(),
        String::from_utf8_lossy(&text)
    );
    if let Ok(p) = std::env::var("DUMP_TEXT_FILE") {
        std::fs::write(p, &text).ok();
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
    crc: u32,
    acc: &mut CaseAccumulator,
) -> Result<(), RomTestError> {
    let serial_output = bus.serial_output();
    cell.verify()
        .verify_serial(serial_output, &mut acc.checks)?;
    cell.verify().verify_frame(crc, &mut acc.checks);
    cell.verify().verify_memory(bus, &mut acc.checks)?;
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

fn step_cycles(bus: &mut GbcMemoryBus, cpu: &mut Lr35902Cpu, cycles: usize) {
    for _ in 0..cycles {
        // T-cycle synchronized: CPU + PPU advance at 1 T-cycle per call.
        // 4 calls = 1 M-cycle for CPU, 4 T-cycles for PPU.
        for _ in 0..4 {
            bus.step_tcycle(cpu);
        }
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
