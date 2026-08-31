use nerust_gba_core::{cpu_registers::CpuRegisters, memory::GbaMemoryBus};
use serde::{Deserialize, Serialize};

use crate::error::RomTestError;

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct VerifySpec {
    #[serde(default)]
    pub memory: Vec<MemoryEntry>,
    #[serde(default)]
    pub registers: RegisterVerify,
    #[serde(default)]
    pub frame_pixels: Vec<FramePixelEntry>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntry {
    pub address: String,
    pub value: String,
    #[serde(default = "default_width")]
    pub width: u8,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FramePixelEntry {
    pub x: usize,
    pub y: usize,
    pub color: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct RegisterVerify {
    pub r0: Option<String>,
    pub r1: Option<String>,
    pub r2: Option<String>,
    pub r3: Option<String>,
    pub r4: Option<String>,
    pub r5: Option<String>,
    pub r6: Option<String>,
    pub r7: Option<String>,
    pub r8: Option<String>,
    pub r9: Option<String>,
    pub r10: Option<String>,
    pub r11: Option<String>,
    pub r12: Option<String>,
    pub r13: Option<String>,
    pub r14: Option<String>,
    pub r15: Option<String>,
    pub cpsr: Option<String>,
}

impl VerifySpec {
    pub fn is_empty(&self) -> bool {
        self.memory.is_empty() && self.registers.is_empty() && self.frame_pixels.is_empty()
    }

    pub fn validate(&self) -> Result<(), RomTestError> {
        for entry in &self.memory {
            let address = parse_hex(&entry.address)?;
            let value = parse_hex(&entry.value)?;
            if address > u64::from(u32::MAX) || !matches!(entry.width, 1 | 2 | 4) {
                return Err(RomTestError::InvalidManifest(format!(
                    "invalid memory check at {}",
                    entry.address
                )));
            }
            let max = match entry.width {
                1 => u64::from(u8::MAX),
                2 => u64::from(u16::MAX),
                _ => u64::from(u32::MAX),
            };
            if value > max {
                return Err(RomTestError::InvalidManifest(format!(
                    "memory value {} does not fit width {}",
                    entry.value, entry.width
                )));
            }
        }
        for entry in &self.frame_pixels {
            if entry.x >= nerust_gba_core::ppu::WIDTH
                || entry.y >= nerust_gba_core::ppu::HEIGHT
                || parse_hex(&entry.color)? > 0x7FFF
            {
                return Err(RomTestError::InvalidManifest(format!(
                    "invalid frame pixel at {},{}",
                    entry.x, entry.y
                )));
            }
        }
        self.registers.validate()
    }

    pub fn verify(
        &self,
        bus: &mut GbaMemoryBus,
        registers: &CpuRegisters,
    ) -> Result<Vec<CheckResult>, RomTestError> {
        let mut checks = Vec::new();
        for entry in &self.memory {
            let address = parse_hex(&entry.address)? as u32;
            let expected = parse_hex(&entry.value)? as u32;
            let actual = match entry.width {
                1 => u32::from(bus.read8(address)),
                2 => u32::from(bus.read16(address)),
                4 => bus.read32(address),
                _ => unreachable!("width was validated"),
            };
            checks.push(CheckResult {
                name: format!("memory@0x{address:08X}"),
                expected: format!("0x{expected:0width$X}", width = entry.width as usize * 2),
                actual: format!("0x{actual:0width$X}", width = entry.width as usize * 2),
                passed: actual == expected,
            });
        }
        self.registers.verify(registers, &mut checks)?;
        let frame = bus.frame_buffer();
        for entry in &self.frame_pixels {
            let bgr555 = parse_hex(&entry.color)? as u16;
            let expected = nerust_gba_core::ppu::bgr555_to_rgba8888(bgr555);
            let actual = frame[entry.y * nerust_gba_core::ppu::WIDTH + entry.x];
            checks.push(CheckResult {
                name: format!("frame@{},{}", entry.x, entry.y),
                expected: format!("0x{bgr555:04X}"),
                actual: format!(
                    "rgba({:02X}{:02X}{:02X}{:02X})",
                    actual.to_le_bytes()[0],
                    actual.to_le_bytes()[1],
                    actual.to_le_bytes()[2],
                    actual.to_le_bytes()[3]
                ),
                passed: actual == expected,
            });
        }
        Ok(checks)
    }
}

impl RegisterVerify {
    pub fn is_empty(&self) -> bool {
        self.entries().iter().all(|(_, value, _)| value.is_none())
    }

    pub fn validate(&self) -> Result<(), RomTestError> {
        for (name, value, _) in self.entries() {
            if let Some(value) = value
                && parse_hex(value)? > u64::from(u32::MAX)
            {
                return Err(RomTestError::InvalidManifest(format!(
                    "register {name} value is out of range"
                )));
            }
        }
        Ok(())
    }

    pub fn matches(&self, registers: &CpuRegisters) -> bool {
        self.entries().iter().all(|(_, expected, index)| {
            expected.is_none_or(|expected| {
                parse_hex(expected).is_ok_and(|expected| {
                    let actual = index.map_or_else(|| registers.cpsr(), |index| registers.r(index));
                    u64::from(actual) == expected
                })
            })
        })
    }

    fn verify(
        &self,
        registers: &CpuRegisters,
        checks: &mut Vec<CheckResult>,
    ) -> Result<(), RomTestError> {
        for (name, expected, index) in self.entries() {
            let Some(expected) = expected else { continue };
            let expected = parse_hex(expected)? as u32;
            let actual = index.map_or_else(|| registers.cpsr(), |index| registers.r(index));
            checks.push(CheckResult {
                name: format!("register.{name}"),
                expected: format!("0x{expected:08X}"),
                actual: format!("0x{actual:08X}"),
                passed: actual == expected,
            });
        }
        Ok(())
    }

    fn entries(&self) -> [(&'static str, Option<&str>, Option<usize>); 17] {
        [
            ("R0", self.r0.as_deref(), Some(0)),
            ("R1", self.r1.as_deref(), Some(1)),
            ("R2", self.r2.as_deref(), Some(2)),
            ("R3", self.r3.as_deref(), Some(3)),
            ("R4", self.r4.as_deref(), Some(4)),
            ("R5", self.r5.as_deref(), Some(5)),
            ("R6", self.r6.as_deref(), Some(6)),
            ("R7", self.r7.as_deref(), Some(7)),
            ("R8", self.r8.as_deref(), Some(8)),
            ("R9", self.r9.as_deref(), Some(9)),
            ("R10", self.r10.as_deref(), Some(10)),
            ("R11", self.r11.as_deref(), Some(11)),
            ("R12", self.r12.as_deref(), Some(12)),
            ("R13", self.r13.as_deref(), Some(13)),
            ("R14", self.r14.as_deref(), Some(14)),
            ("R15", self.r15.as_deref(), Some(15)),
            ("CPSR", self.cpsr.as_deref(), None),
        ]
    }
}

pub fn parse_hex(value: &str) -> Result<u64, RomTestError> {
    let value = value.trim();
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if digits.is_empty() {
        return Err(RomTestError::InvalidManifest("empty hex value".to_string()));
    }
    u64::from_str_radix(digits, 16)
        .map_err(|_| RomTestError::InvalidManifest(format!("invalid hex value `{value}`")))
}

const fn default_width() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_hex_and_width() {
        let invalid_hex = VerifySpec {
            memory: vec![MemoryEntry {
                address: "nope".into(),
                value: "1".into(),
                width: 1,
            }],
            ..Default::default()
        };
        assert!(invalid_hex.validate().is_err());
        let invalid_width = VerifySpec {
            memory: vec![MemoryEntry {
                address: "0x02000000".into(),
                value: "1".into(),
                width: 3,
            }],
            ..Default::default()
        };
        assert!(invalid_width.validate().is_err());
    }

    #[test]
    fn verifies_memory_and_registers() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x02000000, 0x12345678);
        let mut registers = CpuRegisters::post_bios();
        registers.set_r(0, 0x42);
        let spec: VerifySpec = serde_saphyr::from_str("memory: [{ address: 0x02000000, value: 0x12345678, width: 4 }]\nregisters: { r0: 0x42 }").unwrap();
        spec.validate().unwrap();
        assert!(
            spec.verify(&mut bus, &registers)
                .unwrap()
                .iter()
                .all(|check| check.passed)
        );
    }
}
