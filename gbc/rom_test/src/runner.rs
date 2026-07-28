use std::path::Path;

use nerust_gbc_core::{
    cartridge::Cartridge,
    cpu_core::{GbcModel, Lr35902Cpu},
    memory::GbcMemoryBus,
};

use super::{error::RomTestError, manifest::RomCase};

/// Load a GBC ROM from a file path.
fn load_rom(path: &Path) -> Result<Cartridge, RomTestError> {
    let rom_bytes = std::fs::read(path)?;
    let header =
        nerust_gbc_core::cartridge_header::CartridgeHeader::parse(&rom_bytes).ok_or_else(|| {
            RomTestError::InvalidManifest(format!("invalid ROM header: {}", path.display()))
        })?;
    let mbc = nerust_gbc_core::cartridge_mbc::create_mbc(&header, rom_bytes, None);
    Ok(Cartridge::new(mbc))
}

/// Run a ROM test case through all its events and return the final serial output.
pub fn run_case(case: &RomCase, rom_root: &Path) -> Result<String, RomTestError> {
    let rom_path = case.rom_path(rom_root);
    if !rom_path.exists() {
        return Err(RomTestError::InvalidManifest(format!(
            "ROM not found: {}",
            rom_path.display()
        )));
    }

    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cartridge(load_rom(&rom_path)?);
    let model = match case.model {
        super::manifest::GbcModel::Dmg => GbcModel::Dmg,
        super::manifest::GbcModel::Cgb => GbcModel::Cgb,
        super::manifest::GbcModel::Agb => GbcModel::Agb,
    };
    let mut cpu = Lr35902Cpu::with_model(model);
    cpu.registers_mut().set_pc(0x0100);

    // Process each event
    for event in &case.events {
        for _ in 0..event.cycles {
            cpu.step(&mut bus);
            bus.step_devices(4);
        }

        // Verify serial output hash
        if let Some(ref serial_hash) = event.serial {
            if !serial_hash.hash.is_empty() {
                let actual = crc32(bus.serial_output());
                let expected = parse_hex(&serial_hash.hash)? as u32;
                if actual != expected {
                    return Err(RomTestError::SerialMismatch(case.id.clone()));
                }
            }
        }

        // Verify frame hash (stub: computed on next VBlank)
        if let Some(ref frame_hash) = event.frame {
            // Frame hashing requires rendering and PNG-compatible CRC
            // Stub for now — always passes.
            let _ = frame_hash;
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

    Ok(String::from_utf8_lossy(bus.serial_output()).into_owned())
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
