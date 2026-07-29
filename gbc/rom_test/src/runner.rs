use std::path::Path;

use nerust_gbc_core::{
    cartridge::Cartridge,
    cpu_core::{GbcModel, Lr35902Cpu},
    memory::GbcMemoryBus,
};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use super::{error::RomTestError, manifest::RomCase, media};

/// Read ROM bytes and determine effective model based on header CGB flag.
fn effective_model(case: &RomCase, rom_path: &Path) -> Result<GbcModel, RomTestError> {
    let rom_bytes = std::fs::read(rom_path).map_err(RomTestError::Io)?;
    let cgb_flag = rom_bytes.get(0x143).copied().unwrap_or(0);
    let requested = match case.model {
        super::manifest::GbcModel::Dmg => GbcModel::Dmg,
        super::manifest::GbcModel::Cgb => GbcModel::Cgb,
        super::manifest::GbcModel::Agb => GbcModel::Agb,
    };
    // Auto-downgrade: DMG-only ROM ($00) on CGB/AGB → DMG mode
    if cgb_flag == 0x00 && (requested == GbcModel::Cgb || requested == GbcModel::Agb) {
        return Ok(GbcModel::Dmg);
    }
    // Auto-upgrade: CGB-only ROM ($C0) on DMG → CGB mode
    if cgb_flag == 0xC0 && requested == GbcModel::Dmg {
        return Ok(GbcModel::Cgb);
    }
    Ok(requested)
}

/// Run a ROM test case through all its events and return the serial output,
/// plus paths to any captured screenshots.
pub fn run_case(
    case: &RomCase,
    rom_root: &Path,
    screenshots_dir: Option<&Path>,
) -> Result<(String, Vec<String>), RomTestError> {
    let rom_path = case.rom_path(rom_root);
    if !rom_path.exists() {
        return Err(RomTestError::InvalidManifest(format!(
            "ROM not found: {}",
            rom_path.display()
        )));
    }

    let model = effective_model(case, &rom_path)?;
    let rom_bytes = std::fs::read(&rom_path)?;
    let header =
        nerust_gbc_core::cartridge_header::CartridgeHeader::parse(&rom_bytes).ok_or_else(|| {
            RomTestError::InvalidManifest(format!("invalid ROM header: {}", rom_path.display()))
        })?;
    let mbc = nerust_gbc_core::cartridge_mbc::create_mbc(&header, rom_bytes, None);

    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cartridge(Cartridge::new(mbc));
    // CGB mode requires both CGB HARDWARE and a CGB-aware ROM.
    // DMG-only games on CGB hardware must render identically to DMG.
    let hw_is_cgb = match case.model {
        super::manifest::GbcModel::Cgb | super::manifest::GbcModel::Agb => true,
        super::manifest::GbcModel::Dmg => false,
    };
    let rom_is_cgb = header.cgb_flag & 0x80 != 0;
    bus.set_cgb_mode(hw_is_cgb && rom_is_cgb);
    let mut cpu = Lr35902Cpu::with_model(model);
    cpu.registers_mut().set_pc(0x0100);

    // Process each event
    let mut screenshots: Vec<String> = Vec::new();
    for (event_idx, event) in case.events.iter().enumerate() {
        for _ in 0..event.cycles {
            cpu.step(&mut bus);
            bus.step_devices(4);
        }

        // Compute PNG screenshot data (for both file save and hash check)
        let mut fb = FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba);
        fb.resize(160, 144);
        bus.render_frame(&mut fb);
        let png_data = media::encode_screenshot_png(&fb)?;

        // Compute frame hash from PNG data (deterministic for same pixels)
        let frame_crc = crc32(&png_data);

        // Save screenshot to file if requested
        if let Some(screenshots_dir) = screenshots_dir {
            let shot_name = format!("{}_{}.png", case.id, event_idx);
            let shot_path = screenshots_dir.join(&shot_name);
            std::fs::write(&shot_path, &png_data).map_err(RomTestError::Io)?;
            screenshots.push(shot_name);
        }

        // Verify serial output hash
        if let Some(serial_hash) = event.serial.as_ref().filter(|s| !s.hash.is_empty()) {
            let actual = crc32(bus.serial_output());
            let expected = parse_hex(&serial_hash.hash)? as u32;
            if actual != expected {
                return Err(RomTestError::SerialMismatch(case.id.clone()));
            }
        }

        // Verify frame hash (CRC32 of raw RGBA frame buffer)
        if let Some(ref frame_hash) = event.frame {
            if !frame_hash.hash.is_empty() {
                let expected = parse_hex(&frame_hash.hash)? as u32;
                if frame_crc != expected {
                    return Err(RomTestError::CaseFailed(
                        case.id.clone(),
                        format!(
                            "frame hash: expected {:08X}, got {:08X}",
                            expected, frame_crc
                        ),
                    ));
                }
            }
        }

        // Verify audio hash (stub)
        if let Some(ref audio_hash) = event.audio {
            let _ = audio_hash;
        }

        // Verify memory values
        if let Some(ref memory) = event.memory {
            for entry in memory {
                let addr = parse_hex(&entry.address)? as u16;
                let expected = parse_hex(&entry.value)? as u8;
                let actual = bus.read(addr);
                if actual != expected {
                    return Err(RomTestError::CaseFailed(
                        case.id.clone(),
                        format!(
                            "memory at ${:04X}: expected ${:02X}, got ${:02X}",
                            addr, expected, actual
                        ),
                    ));
                }
            }
        }

        // Apply pad input
        if let Some(ref pad) = event.pad {
            let mut joypad = 0xFFu8;
            for entry in pad {
                // GBC joypad: bits 0-3 = direction/button, bit 4=select, bit 5=select
                // 0 = pressed, 1 = released
                let mask = match entry.button.as_str() {
                    "right" => !0x01,
                    "left" => !0x02,
                    "up" => !0x04,
                    "down" => !0x08,
                    "a" => !0x01,
                    "b" => !0x02,
                    "select" => !0x04,
                    "start" => !0x08,
                    _ => 0xFF,
                };
                let pressed = entry.state == "press";
                if pressed {
                    if entry.button == "a"
                        || entry.button == "b"
                        || entry.button == "select"
                        || entry.button == "start"
                    {
                        joypad &= mask & 0x0F; // button keys: bits 0-3
                    } else {
                        joypad &= mask & 0xF0; // direction keys: bits 4-7
                    }
                }
            }
            bus.set_joypad(joypad);
        }
    }

    Ok((
        String::from_utf8_lossy(bus.serial_output()).into_owned(),
        screenshots,
    ))
}

fn parse_hex(s: &str) -> Result<u64, RomTestError> {
    let s = s.trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(s, 16)
        .map_err(|_| RomTestError::InvalidManifest(format!("invalid hex value: {}", s)))
}

fn crc32(data: &[u8]) -> u32 {
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let mut digest = crc.digest();
    digest.update(data);
    digest.finalize()
}
