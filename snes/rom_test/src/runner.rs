use crate::manifest::{Assertion, ManifestError, RomCase};
use crate::media::{encode_screenshot_png, load_png_rgba, png_hash_from_path, screen_hash_rgba};
use crate::results::{CaseOutcome, Validation, ValidationOptions};
use nerust_snes_core::{Core, CpuState};
use nerust_snes_render::{RenderContext, render_screen};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub fn validate_case(case: &RomCase) -> CaseOutcome {
    validate_case_with_options(case, ValidationOptions::testing())
}

pub fn validate_case_with_options(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    let should_wait_for_final_screen =
        case.expected_screen_hash.is_some() || case.png_path().is_some();
    let mut core = match load_core_for_case(case) {
        Ok(core) => core,
        Err(error) => return internal_error(case, error),
    };

    let mut steps_executed = 0_u64;
    let mut next_reset_index = 0_usize;

    while steps_executed < case.max_steps {
        // Check for scheduled reset before stepping
        if let Some(&reset_at) = case.reset_at_steps.get(next_reset_index)
            && steps_executed == reset_at
        {
            core.reset_cpu();
            next_reset_index += 1;
        }

        match core.step() {
            Ok(()) => {
                steps_executed += 1;
            }
            Err(error) => {
                return internal_error(
                    case,
                    format!("core error after {steps_executed} steps: {error}"),
                );
            }
        }

        if steps_executed.is_multiple_of(case.check_interval_steps) {
            match assertion_failures(case, &core) {
                Ok(failures) if failures.is_empty() && !should_wait_for_final_screen => {
                    return finalize_validation(case, steps_executed, failures, &core, options);
                }
                Ok(_) => {}
                Err(error) => {
                    return internal_error(case, error.to_string());
                }
            }
        }
    }

    match assertion_failures(case, &core) {
        Ok(mut failures) => {
            if !failures.is_empty() {
                let reason = if core.current_state() == CpuState::Stopped {
                    format!("core stopped after {steps_executed} steps before expectations matched")
                } else {
                    format!("expectations did not match within {} steps", case.max_steps)
                };
                failures.insert(0, reason);
            }
            finalize_validation(case, steps_executed, failures, &core, options)
        }
        Err(error) => internal_error(case, error.to_string()),
    }
}

pub fn load_core_for_case(case: &RomCase) -> Result<Core, String> {
    let rom = fs::read(case.rom_path()).map_err(|error| {
        format!(
            "failed to read ROM `{}`: {error}",
            case.rom_path().display()
        )
    })?;
    let msu1_sidecars = load_msu1_sidecars(case.rom_path())?;

    Core::from_rom_bytes_with_msu1_sidecars(
        &rom,
        msu1_sidecars.data.as_deref(),
        &msu1_sidecars.audio_tracks,
    )
    .map_err(|error| {
        format!(
            "failed to construct SNES core from `{}`: {error}",
            case.rom_path().display()
        )
    })
}

struct Msu1Sidecars {
    data: Option<Vec<u8>>,
    audio_tracks: Vec<u16>,
}

fn load_msu1_sidecars(rom_path: &Path) -> Result<Msu1Sidecars, String> {
    Ok(Msu1Sidecars {
        data: load_msu1_data_sidecar(rom_path)?,
        audio_tracks: discover_msu1_audio_tracks(rom_path)?,
    })
}

fn load_msu1_data_sidecar(rom_path: &Path) -> Result<Option<Vec<u8>>, String> {
    // Try .msu file first (uncompressed sidecar).
    let data_path = rom_path.with_extension("msu");
    match fs::read(&data_path) {
        Ok(bytes) => return Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to read MSU-1 data sidecar `{}`: {error}",
                data_path.display()
            ));
        }
    }

    // Try .msu.7z file (compressed sidecar, decompress to memory).
    let compressed_path = rom_path.with_extension("msu.7z");
    let compressed = match fs::read(&compressed_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read MSU-1 compressed sidecar `{}`: {error}",
                compressed_path.display()
            ));
        }
    };
    decompress_msu_7z(&compressed).map(Some)
}

/// Decompress a 7z archive containing an MSU-1 data file.
///
/// The archive is expected to hold a single file (the .msu data).
/// Its contents are decompressed entirely to memory and returned.
fn decompress_msu_7z(compressed: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    let cursor = Cursor::new(compressed);
    let mut reader = sevenz_rust2::ArchiveReader::new(cursor, sevenz_rust2::Password::empty())
        .map_err(|e| format!("failed to open MSU-1 7z archive: {e}"))?;

    let mut result: Option<Vec<u8>> = None;
    reader
        .for_each_entries(|_entry, entry_reader| {
            let mut data = Vec::new();
            entry_reader.read_to_end(&mut data)?;
            result = Some(data);
            // Stop after the first entry (a 7z archive should contain a single file).
            Ok(false)
        })
        .map_err(|e| format!("failed to decompress MSU-1 7z archive: {e}"))?;

    result.ok_or_else(|| "MSU-1 7z archive contains no files".to_string())
}

pub fn discover_msu1_audio_tracks(rom_path: &Path) -> Result<Vec<u16>, String> {
    let Some(stem) = rom_path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(Vec::new());
    };
    let prefix = format!("{stem}-");
    let directory = rom_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to scan MSU-1 audio sidecars in `{}`: {error}",
            directory.display()
        )
    })?;
    let mut tracks = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to scan MSU-1 audio sidecars in `{}`: {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pcm"))
        {
            continue;
        }

        let Some(file_stem) = path.file_stem().and_then(|file_stem| file_stem.to_str()) else {
            continue;
        };
        if let Some(track) = file_stem
            .strip_prefix(&prefix)
            .and_then(|track| track.parse::<u16>().ok())
        {
            tracks.push(track);
        }
    }
    tracks.sort_unstable();
    tracks.dedup();
    Ok(tracks)
}

pub fn has_msu1_data_sidecar(rom_path: &Path) -> Result<bool, String> {
    let data_path = rom_path.with_extension("msu");
    match fs::metadata(&data_path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "failed to stat MSU-1 data sidecar `{}`: {error}",
            data_path.display()
        )),
    }
}

fn finalize_validation(
    case: &RomCase,
    steps_executed: u64,
    mut failures: Vec<String>,
    core: &Core,
    options: ValidationOptions,
) -> CaseOutcome {
    let mut ctx = RenderContext::new();
    match render_screen(core, &mut ctx) {
        Ok(()) => (),
        Err(error) => {
            return internal_error(
                case,
                format!("failed to render final screen after {steps_executed} steps: {error}"),
            );
        }
    };
    let final_screen_hash = screen_hash_rgba(ctx.frame.as_ref());
    if let Some(png_path) = case.png_path() {
        match png_hash_from_path(png_path) {
            Ok(png_hash) if png_hash != final_screen_hash => {
                failures.push(format!(
                    "screen_hash: reference PNG 0x{png_hash:016X}, rendered 0x{final_screen_hash:016X}"
                ));
                if let Ok(png_rgba) = load_png_rgba(png_path) {
                    let our_pitch = ctx.frame.width() * 4;
                    let png_pitch = png_rgba
                        .len()
                        .checked_div(ctx.frame.height())
                        .unwrap_or(our_pitch);
                    for y in 0..ctx.frame.height() {
                        for x in 0..ctx.frame.width() {
                            let png_idx = y * png_pitch + x * 4;
                            let our_idx = y * our_pitch + x * 4;
                            let rgba = ctx.frame.as_ref();
                            if png_idx + 4 <= png_rgba.len() && our_idx + 4 <= rgba.len() {
                                let pr = png_rgba[png_idx];
                                let pg = png_rgba[png_idx + 1];
                                let pb = png_rgba[png_idx + 2];
                                let pa = png_rgba[png_idx + 3];
                                let or = rgba[our_idx];
                                let og = rgba[our_idx + 1];
                                let ob = rgba[our_idx + 2];
                                let oa = rgba[our_idx + 3];
                                if (pr, pg, pb, pa) != (or, og, ob, oa) {
                                    let screen_x = x;
                                    let screen_y = y;
                                    failures.push(format!(
                                        "first pixel diff at ({screen_x}, {screen_y}): PNG ({pr},{pg},{pb},{pa}), rendered ({or},{og},{ob},{oa})"
                                    ));
                                    break;
                                }
                            }
                        }
                        if failures
                            .last()
                            .is_some_and(|f| f.starts_with("first pixel diff"))
                        {
                            break;
                        }
                    }
                    if !failures.iter().any(|f| f.starts_with("first pixel diff")) {
                        failures.push(format!(
                            "pixel diff: different sizes? PNG {} bytes, rendered {}x{}",
                            png_rgba.len(),
                            ctx.frame.width(),
                            ctx.frame.height(),
                        ));
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                return internal_error(
                    case,
                    format!(
                        "failed to hash reference PNG `{}`: {error}",
                        png_path.display()
                    ),
                );
            }
        }
    } else if let Ok(Some(expected_screen_hash)) = case.expected_screen_hash()
        && expected_screen_hash != final_screen_hash
    {
        failures.push(format!(
            "screen_hash: expected 0x{expected_screen_hash:016X}, got 0x{final_screen_hash:016X}"
        ));
    }
    let screenshot_png = if options.capture_screenshot_png {
        match encode_screenshot_png(
            ctx.frame.as_ref(),
            ctx.frame.width() as u32,
            ctx.frame.height() as u32,
        ) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                return internal_error(
                    case,
                    format!(
                        "failed to encode final screenshot after {steps_executed} steps: {error}"
                    ),
                );
            }
        }
    } else {
        None
    };

    CaseOutcome::Completed(Validation {
        case_id: case.id.clone(),
        description: case.description.clone(),
        rom: case.rom_path().display().to_string(),
        steps_executed,
        final_screen_hash,
        screenshot_png,
        failures,
    })
}

fn internal_error(case: &RomCase, message: String) -> CaseOutcome {
    CaseOutcome::InternalError {
        case_id: case.id.clone(),
        description: case.description.clone(),
        rom: case.rom_path().display().to_string(),
        message,
    }
}

fn assertion_failures(case: &RomCase, core: &Core) -> Result<Vec<String>, ManifestError> {
    case.assertions
        .iter()
        .map(|assertion| match assertion {
            Assertion::BusU8 { .. }
            | Assertion::ApuRamU8 { .. }
            | Assertion::WramU8 { .. }
            | Assertion::VramU8 { .. }
            | Assertion::CgramU8 { .. }
            | Assertion::OamU8 { .. } => evaluate_u8_assertion(assertion, core),
            Assertion::BusU16 { .. }
            | Assertion::ApuRamU16 { .. }
            | Assertion::WramU16 { .. }
            | Assertion::VramU16 { .. }
            | Assertion::CgramU16 { .. }
            | Assertion::OamU16 { .. } => evaluate_u16_assertion(assertion, core),
        })
        .filter_map(Result::transpose)
        .collect()
}

fn evaluate_u8_assertion(
    assertion: &Assertion,
    core: &Core,
) -> Result<Option<String>, ManifestError> {
    let address = assertion.address()?;
    let expected = assertion.expected_u8()?;
    let actual = match assertion {
        Assertion::BusU8 { .. } => core.peek(address),
        Assertion::ApuRamU8 { .. } => core.peek_apu_ram(address as u16),
        Assertion::WramU8 { .. } => core.peek_wram(address as usize),
        Assertion::VramU8 { .. } => core.peek_vram(address as usize),
        Assertion::CgramU8 { .. } => core.peek_cgram(address as usize),
        Assertion::OamU8 { .. } => core.peek_oam(address as usize),
        _ => {
            return Err(ManifestError::Invalid {
                message: "evaluate_u8_assertion called for 16-bit assertion".to_string(),
            });
        }
    };

    if actual == expected {
        Ok(None)
    } else {
        Ok(Some(format!(
            "{} @ 0x{address:06X}: expected 0x{expected:02X}, got 0x{actual:02X}",
            assertion_kind(assertion)
        )))
    }
}

fn evaluate_u16_assertion(
    assertion: &Assertion,
    core: &Core,
) -> Result<Option<String>, ManifestError> {
    let address = assertion.address()?;
    let expected = assertion.expected_u16()?;
    let actual = match assertion {
        Assertion::BusU16 { .. } => {
            u16::from_le_bytes([core.peek(address), core.peek(address + 1)])
        }
        Assertion::ApuRamU16 { .. } => u16::from_le_bytes([
            core.peek_apu_ram(address as u16),
            core.peek_apu_ram((address as u16) + 1),
        ]),
        Assertion::WramU16 { .. } => u16::from_le_bytes([
            core.peek_wram(address as usize),
            core.peek_wram(address as usize + 1),
        ]),
        Assertion::VramU16 { .. } => u16::from_le_bytes([
            core.peek_vram(address as usize),
            core.peek_vram(address as usize + 1),
        ]),
        Assertion::CgramU16 { .. } => u16::from_le_bytes([
            core.peek_cgram(address as usize),
            core.peek_cgram(address as usize + 1),
        ]),
        Assertion::OamU16 { .. } => u16::from_le_bytes([
            core.peek_oam(address as usize),
            core.peek_oam(address as usize + 1),
        ]),
        _ => {
            return Err(ManifestError::Invalid {
                message: "evaluate_u16_assertion called for 8-bit assertion".to_string(),
            });
        }
    };

    if actual == expected {
        Ok(None)
    } else {
        Ok(Some(format!(
            "{} @ 0x{address:06X}: expected 0x{expected:04X}, got 0x{actual:04X}",
            assertion_kind(assertion)
        )))
    }
}

fn assertion_kind(assertion: &Assertion) -> &'static str {
    match assertion {
        Assertion::BusU8 { .. } => "bus_u8",
        Assertion::BusU16 { .. } => "bus_u16",
        Assertion::ApuRamU8 { .. } => "apu_ram_u8",
        Assertion::ApuRamU16 { .. } => "apu_ram_u16",
        Assertion::WramU8 { .. } => "wram_u8",
        Assertion::WramU16 { .. } => "wram_u16",
        Assertion::VramU8 { .. } => "vram_u8",
        Assertion::VramU16 { .. } => "vram_u16",
        Assertion::CgramU8 { .. } => "cgram_u8",
        Assertion::CgramU16 { .. } => "cgram_u16",
        Assertion::OamU8 { .. } => "oam_u8",
        Assertion::OamU16 { .. } => "oam_u16",
    }
}

#[cfg(test)]
mod tests {
    use super::{discover_msu1_audio_tracks, validate_case};
    use crate::manifest::load_manifest;
    use crate::results::CaseOutcome;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn unique_temp_path(name: &str, extension: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("nerust-snes-rom-test-{name}-{unique}.{extension}"))
    }

    fn write_test_rom(path: &PathBuf) {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST HASH ROM        ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2].copy_from_slice(&0x8000u16.to_le_bytes());
        rom[0x0000] = 0xEA;
        fs::write(path, rom).expect("test rom should be written");
    }

    #[test]
    fn msu1_audio_discovery_matches_decimal_pcm_tracks() {
        let directory = unique_temp_path("msu1-audio-discovery", "dir");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("temporary directory should be created");
        fs::write(directory.join("Game-1.pcm"), []).expect("pcm file should be written");
        fs::write(directory.join("Game-3.PCM"), []).expect("pcm file should be written");
        fs::write(directory.join("Game-0012.pcm"), []).expect("pcm file should be written");
        fs::write(directory.join("Game-12.pcm"), []).expect("pcm file should be written");
        fs::write(directory.join("Game-70000.pcm"), []).expect("pcm file should be written");
        fs::write(directory.join("Other-2.pcm"), []).expect("pcm file should be written");
        fs::write(directory.join("Game.msu"), []).expect("msu data file should be written");

        let tracks = discover_msu1_audio_tracks(&directory.join("Game.sfc"))
            .expect("audio track discovery should succeed");

        let _ = fs::remove_dir_all(&directory);
        assert_eq!(tracks, [1, 3, 12]);
    }

    #[test]
    fn expected_screen_hash_mismatch_is_reported_as_a_failure() {
        let rom_path = unique_temp_path("hash-mismatch", "sfc");
        let manifest_path = unique_temp_path("hash-mismatch", "yaml");
        write_test_rom(&rom_path);
        fs::write(
            &manifest_path,
            format!(
                "rom_root: .\ncases:\n  - id: hash-mismatch\n    description: Hash mismatch test\n    rom: {}\n    max_steps: 32\n    check_interval_steps: 16\n    expected_screen_hash: \"0x0000000000000000\"\n",
                rom_path.display()
            ),
        )
        .expect("manifest should be written");

        let manifest = load_manifest(&manifest_path).expect("manifest should load");
        let outcome = validate_case(manifest.case("hash-mismatch").expect("case should exist"));

        match outcome {
            CaseOutcome::Completed(validation) => {
                assert_eq!(validation.steps_executed, 32);
                assert!(
                    validation
                        .failures
                        .iter()
                        .any(|failure| failure.starts_with("screen_hash: expected")),
                    "expected screen hash failure, got {:?}",
                    validation.failures
                );
            }
            other => panic!("expected completed validation result, got {other:?}"),
        }

        fs::remove_file(rom_path).ok();
        fs::remove_file(manifest_path).ok();
    }
}
