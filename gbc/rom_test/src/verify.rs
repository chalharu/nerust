use serde::{Deserialize, Serialize};

use nerust_gbc_core::{cpu_registers::CpuRegisters, memory::GbcMemoryBus};

use super::{error::RomTestError, media};

type RegisterEntry<'a> = (&'static str, Option<&'a str>, fn(&CpuRegisters) -> u8);

/// Outcome of one declared verification check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
}

/// Expected memory value at a given address.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryEntry {
    pub address: String, // hex, e.g. "0xC000"
    pub value: String,   // hex, e.g. "0x42"
}

/// Repeated expected byte values, optionally arranged in strided rows.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRegion {
    pub start: String,
    pub length: usize,
    pub value: String,
    #[serde(default)]
    pub row_length: Option<usize>,
    #[serde(default)]
    pub stride: Option<usize>,
}

/// Compare two 8-bit memory locations. Useful for ROM protocols that publish
/// an actual value and its expected counterpart separately.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryComparison {
    pub actual_address: String,
    pub expected_address: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub when: Option<MemoryCondition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryCondition {
    pub address: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub not_value: Option<String>,
}

/// Expected values for the 8-bit CPU registers used by test ROM protocols.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegisterVerify {
    #[serde(default)]
    pub b: Option<String>,
    #[serde(default)]
    pub c: Option<String>,
    #[serde(default)]
    pub d: Option<String>,
    #[serde(default)]
    pub e: Option<String>,
    #[serde(default)]
    pub h: Option<String>,
    #[serde(default)]
    pub l: Option<String>,
}

impl RegisterVerify {
    pub fn is_empty(&self) -> bool {
        self.entries().iter().all(|(_, value, _)| value.is_none())
    }

    pub fn validate(&self) -> Result<(), RomTestError> {
        for (name, value, _) in self.entries() {
            if value.is_some_and(|value| parse_hex(value).is_err()) {
                return Err(RomTestError::InvalidManifest(format!(
                    "invalid CPU register {name} value"
                )));
            }
            if value
                .is_some_and(|value| parse_hex(value).is_ok_and(|value| value > u64::from(u8::MAX)))
            {
                return Err(RomTestError::InvalidManifest(format!(
                    "CPU register {name} value is out of range"
                )));
            }
        }
        Ok(())
    }

    pub fn matches(&self, registers: &CpuRegisters) -> bool {
        self.entries().into_iter().all(|(_, expected, actual)| {
            expected.is_none_or(|expected| {
                parse_hex(expected).is_ok_and(|expected| actual(registers) as u64 == expected)
            })
        })
    }

    fn entries(&self) -> [RegisterEntry<'_>; 6] {
        [
            ("B", self.b.as_deref(), CpuRegisters::b),
            ("C", self.c.as_deref(), CpuRegisters::c),
            ("D", self.d.as_deref(), CpuRegisters::d),
            ("E", self.e.as_deref(), CpuRegisters::e),
            ("H", self.h.as_deref(), CpuRegisters::h),
            ("L", self.l.as_deref(), CpuRegisters::l),
        ]
    }

    fn verify(&self, registers: &CpuRegisters, checks: &mut Vec<CheckResult>) {
        for (name, expected, actual) in self.entries() {
            let Some(expected) = expected else {
                continue;
            };
            let expected = parse_hex(expected).expect("register values are validated") as u8;
            let actual = actual(registers);
            checks.push(CheckResult {
                name: format!("register.{name}"),
                expected: format!("${expected:02X}"),
                actual: format!("${actual:02X}"),
                passed: actual == expected,
            });
        }
    }
}

/// Hash algorithm used for a digest check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    #[default]
    Crc32,
    Sha256,
}

/// A digest with an explicit algorithm.
#[derive(Debug, Clone, Deserialize)]
pub struct HashSpec {
    #[serde(default)]
    pub algo: HashAlgo,
    pub value: String,
}

/// Expected digest. Accepts a plain hex string (CRC32 by default) or an
/// object with an explicit `algo`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Hash {
    Plain(String),
    Full(HashSpec),
}

impl Hash {
    pub fn algo(&self) -> HashAlgo {
        match self {
            Hash::Plain(_) => HashAlgo::Crc32,
            Hash::Full(spec) => spec.algo,
        }
    }

    pub fn value(&self) -> &str {
        match self {
            Hash::Plain(v) | Hash::Full(HashSpec { value: v, .. }) => v,
        }
    }
}

/// Serial output verification.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SerialVerify {
    /// CRC32 (or SHA256) of the full serial output.
    #[serde(default)]
    pub hash: Option<Hash>,
    /// Hex-encoded byte suffix the serial output must end with.
    #[serde(default)]
    pub suffix: Option<String>,
    /// Substrings the serial output must contain (any match succeeds).
    #[serde(default)]
    pub contains: Vec<String>,
}

/// Frame (screen) digest verification.
#[derive(Debug, Clone, Deserialize)]
pub struct FrameVerify {
    /// CRC32 of the raw rendered frame.
    pub hash: Hash,
}

/// Declarative verification for one test cell.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VerifySpec {
    #[serde(default)]
    pub serial: Option<SerialVerify>,
    #[serde(default)]
    pub frame: Option<FrameVerify>,
    #[serde(default)]
    pub memory: Vec<MemoryEntry>,
    #[serde(default)]
    pub memory_regions: Vec<MemoryRegion>,
    #[serde(default)]
    pub memory_comparisons: Vec<MemoryComparison>,
    #[serde(default)]
    pub registers: RegisterVerify,
}

impl VerifySpec {
    pub fn has_serial_hash(&self) -> bool {
        self.serial
            .as_ref()
            .is_some_and(|serial| serial.hash.is_some())
    }

    pub fn serial_hash_matches(&self, output: &[u8]) -> bool {
        let Some(hash) = self.serial.as_ref().and_then(|serial| serial.hash.as_ref()) else {
            return false;
        };
        let actual = match hash.algo() {
            HashAlgo::Crc32 => format!("{:08X}", crc32(output)),
            HashAlgo::Sha256 => sha256_hex(output),
        };
        actual.eq_ignore_ascii_case(&normalize_hex(hash.value()))
    }

    /// Validate that all declared hex values parse and fit their targets.
    /// Called at manifest load time so configuration errors fail fast
    /// instead of surfacing per cell at run time.
    pub fn validate(&self) -> Result<(), RomTestError> {
        self.validate_media()?;
        for entry in &self.memory {
            validate_memory_entry(entry)?;
        }
        for region in &self.memory_regions {
            validate_memory_region(region)?;
        }
        for comparison in &self.memory_comparisons {
            validate_memory_comparison(comparison)?;
        }
        self.registers.validate()?;
        Ok(())
    }

    fn validate_media(&self) -> Result<(), RomTestError> {
        if let Some(hash) = self.serial.as_ref().and_then(|serial| serial.hash.as_ref()) {
            parse_hex_bytes(hash.value())?;
        }
        if let Some(suffix) = self
            .serial
            .as_ref()
            .and_then(|serial| serial.suffix.as_ref())
        {
            parse_hex_bytes(suffix)?;
        }
        if let Some(frame) = &self.frame {
            parse_hex_bytes(frame.hash.value())?;
        }
        Ok(())
    }

    /// Check serial output against the hash / suffix / contains expectations.
    pub fn verify_serial(
        &self,
        output: &[u8],
        checks: &mut Vec<CheckResult>,
    ) -> Result<(), RomTestError> {
        let Some(serial) = &self.serial else {
            return Ok(());
        };
        if let Some(hash) = &serial.hash {
            let actual = match hash.algo() {
                HashAlgo::Crc32 => format!("{:08X}", crc32(output)),
                HashAlgo::Sha256 => sha256_hex(output),
            };
            let expected = normalize_hex(hash.value());
            let passed = actual.eq_ignore_ascii_case(&expected);
            checks.push(CheckResult {
                name: "serial_hash".to_string(),
                expected,
                actual,
                passed,
            });
        }
        if let Some(suffix) = &serial.suffix {
            let expected = parse_hex_bytes(suffix)?;
            let passed = output.ends_with(&expected);
            let actual = if passed {
                String::new()
            } else {
                output
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join("")
            };
            checks.push(CheckResult {
                name: "serial_suffix".to_string(),
                expected: suffix.to_string(),
                actual,
                passed,
            });
        }
        for needle in &serial.contains {
            let passed = output
                .windows(needle.len())
                .any(|window| window == needle.as_bytes());
            checks.push(CheckResult {
                name: "serial_contains".to_string(),
                expected: needle.clone(),
                actual: if passed {
                    String::new()
                } else {
                    String::from_utf8_lossy(output).into_owned()
                },
                passed,
            });
        }
        Ok(())
    }

    /// Check the rendered frame digest.
    pub fn verify_frame(&self, actual_crc: u32, checks: &mut Vec<CheckResult>) {
        let Some(frame) = &self.frame else {
            return;
        };
        let expected = normalize_hex(frame.hash.value());
        let actual = format!("{:08X}", actual_crc);
        let passed = actual.eq_ignore_ascii_case(&expected);
        checks.push(CheckResult {
            name: "frame_hash".to_string(),
            expected,
            actual,
            passed,
        });
    }

    /// Check expected memory values.
    pub fn verify_memory(
        &self,
        bus: &GbcMemoryBus,
        checks: &mut Vec<CheckResult>,
    ) -> Result<(), RomTestError> {
        for entry in &self.memory {
            let addr = parse_hex(&entry.address)? as u16;
            let expected = parse_hex(&entry.value)? as u8;
            let actual = bus.debug_read(addr);
            checks.push(CheckResult {
                name: format!("memory@${:04X}", addr),
                expected: format!("${:02X}", expected),
                actual: format!("${:02X}", actual),
                passed: actual == expected,
            });
        }
        for region in &self.memory_regions {
            let start = parse_hex(&region.start)? as usize;
            let expected = parse_hex(&region.value)? as u8;
            let row_length = region.row_length.unwrap_or(region.length);
            let stride = region.stride.unwrap_or(row_length);
            let mismatch = (0..region.length).find_map(|index| {
                let address = start + index / row_length * stride + index % row_length;
                let actual = bus.debug_read(address as u16);
                (actual != expected).then_some((address, actual))
            });
            checks.push(CheckResult {
                name: format!("memory_region@${start:04X}"),
                expected: format!("{} bytes of ${expected:02X}", region.length),
                actual: mismatch.map_or_else(
                    || "all bytes matched".to_string(),
                    |(address, actual)| format!("${actual:02X} at ${address:04X}"),
                ),
                passed: mismatch.is_none(),
            });
        }
        for comparison in &self.memory_comparisons {
            if let Some(condition) = &comparison.when {
                let address = parse_hex(&condition.address)? as u16;
                let actual = bus.debug_read(address);
                let matches = condition
                    .value
                    .as_deref()
                    .map(|value| parse_hex(value).map(|value| actual == value as u8))
                    .or_else(|| {
                        condition
                            .not_value
                            .as_deref()
                            .map(|value| parse_hex(value).map(|value| actual != value as u8))
                    })
                    .transpose()?
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            let actual_address = parse_hex(&comparison.actual_address)? as u16;
            let expected_address = parse_hex(&comparison.expected_address)? as u16;
            let actual = bus.debug_read(actual_address);
            let expected = bus.debug_read(expected_address);
            checks.push(CheckResult {
                name: comparison.name.clone().unwrap_or_else(|| {
                    format!("memory@${actual_address:04X} == memory@${expected_address:04X}")
                }),
                expected: format!("${expected:02X}"),
                actual: format!("${actual:02X}"),
                passed: actual == expected,
            });
        }
        Ok(())
    }

    pub fn verify_registers(&self, registers: &CpuRegisters, checks: &mut Vec<CheckResult>) {
        self.registers.verify(registers, checks);
    }
}

fn validate_memory_entry(entry: &MemoryEntry) -> Result<(), RomTestError> {
    if parse_hex(&entry.address)? > u64::from(u16::MAX) {
        return Err(RomTestError::InvalidManifest(format!(
            "invalid memory address: {}",
            entry.address
        )));
    }
    if parse_hex(&entry.value)? > u64::from(u8::MAX) {
        return Err(RomTestError::InvalidManifest(format!(
            "invalid memory value: {}",
            entry.value
        )));
    }
    Ok(())
}

fn validate_memory_region(region: &MemoryRegion) -> Result<(), RomTestError> {
    let start = parse_hex(&region.start)?;
    let value = parse_hex(&region.value)?;
    let row_length = region.row_length.unwrap_or(region.length);
    let stride = region.stride.unwrap_or(row_length);
    if region.length == 0 || row_length == 0 || stride < row_length {
        return Err(RomTestError::InvalidManifest(
            "memory region dimensions must be positive and stride must cover a row".to_string(),
        ));
    }
    let rows = region.length.div_ceil(row_length);
    let last = start + ((rows - 1) * stride + (region.length - 1) % row_length) as u64;
    if last > u64::from(u16::MAX) || value > u64::from(u8::MAX) {
        return Err(RomTestError::InvalidManifest(
            "memory region exceeds the address or byte value range".to_string(),
        ));
    }
    Ok(())
}

fn validate_memory_comparison(comparison: &MemoryComparison) -> Result<(), RomTestError> {
    for address in [&comparison.actual_address, &comparison.expected_address] {
        if parse_hex(address)? > u64::from(u16::MAX) {
            return Err(RomTestError::InvalidManifest(format!(
                "invalid memory comparison address: {address}"
            )));
        }
    }
    let Some(condition) = &comparison.when else {
        return Ok(());
    };
    if condition.value.is_some() == condition.not_value.is_some() {
        return Err(RomTestError::InvalidManifest(
            "memory comparison condition requires exactly one of value/not_value".to_string(),
        ));
    }
    let value = condition
        .value
        .as_deref()
        .or(condition.not_value.as_deref())
        .expect("condition value presence was validated");
    if parse_hex(&condition.address)? > u64::from(u16::MAX)
        || parse_hex(value)? > u64::from(u8::MAX)
    {
        return Err(RomTestError::InvalidManifest(
            "memory comparison condition is out of range".to_string(),
        ));
    }
    Ok(())
}

/// Raw rendered pixels for reference comparison.
pub struct FramePixels<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

/// Compare a rendered frame against a reference image (exact match only).
///
/// Pushes a `reference` check. On mismatch, returns the PNG bytes of a
/// side-by-side diff image (actual | reference | differences); the caller
/// decides whether and where to persist them. Pure: performs no I/O.
pub fn verify_reference(
    frame: &FramePixels<'_>,
    ref_png: &[u8],
    expected_label: &str,
    checks: &mut Vec<CheckResult>,
) -> Result<Option<Vec<u8>>, RomTestError> {
    let (rw, rh, ref_rgb) = media::decode_png_rgb(ref_png)?;
    let width = frame.width;
    let height = frame.height;
    let expected = expected_label.to_string();

    if rw != width || rh != height {
        checks.push(CheckResult {
            name: "reference".to_string(),
            expected: format!("{expected} ({}x{})", width, height),
            actual: format!("reference is {}x{}", rw, rh),
            passed: false,
        });
        return Ok(None);
    }

    // Drop alpha from the rendered frame for comparison.
    let mut frame_rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for px in frame.rgba.as_chunks::<4>().0 {
        frame_rgb.extend_from_slice(&px[..3]);
    }
    if crc32(&frame_rgb) == crc32(&ref_rgb) {
        checks.push(CheckResult {
            name: "reference".to_string(),
            expected,
            actual: "exact match".to_string(),
            passed: true,
        });
        return Ok(None);
    }

    // Pixel diff: first differing coordinate and count.
    let mut diff_count = 0usize;
    let mut first = None;
    for (i, (a, b)) in frame_rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(ref_rgb.as_chunks::<3>().0.iter())
        .enumerate()
    {
        if a != b {
            if first.is_none() {
                first = Some((i % width as usize, i / width as usize));
            }
            diff_count += 1;
        }
    }
    let (fx, fy) = first.unwrap_or((0, 0));
    let actual = format!("{} differing pixels, first at ({},{})", diff_count, fx, fy);
    checks.push(CheckResult {
        name: "reference".to_string(),
        expected,
        actual,
        passed: false,
    });

    let diff = media::compose_diff_image(&frame_rgb, &ref_rgb, width, height);
    let png = media::encode_rgba_png(width * 3, height, &diff)?;
    Ok(Some(png))
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, RomTestError> {
    let s = s.trim_start_matches("0x").trim_start_matches("0X");
    if !s.len().is_multiple_of(2) {
        return Err(RomTestError::InvalidManifest(format!(
            "invalid hex value: {}",
            s
        )));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| RomTestError::InvalidManifest(format!("invalid hex value: {}", s)))
        })
        .collect()
}

pub(crate) fn parse_hex(s: &str) -> Result<u64, RomTestError> {
    let s = s.trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(s, 16)
        .map_err(|_| RomTestError::InvalidManifest(format!("invalid hex value: {}", s)))
}

fn normalize_hex(s: &str) -> String {
    s.trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_uppercase()
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let mut digest = crc.digest();
    digest.update(data);
    digest.finalize()
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_png(w: u32, h: u32, color: [u8; 3]) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            rgba.extend_from_slice(&color);
            rgba.push(255);
        }
        media::encode_rgba_png(w, h, &rgba).unwrap()
    }

    fn frame(rgba: &[u8], w: u32, h: u32) -> FramePixels<'_> {
        FramePixels {
            rgba,
            width: w,
            height: h,
        }
    }

    fn solid_rgba(w: u32, h: u32, color: [u8; 3]) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            rgba.extend_from_slice(&color);
            rgba.push(255);
        }
        rgba
    }

    #[test]
    fn reference_exact_match() {
        let png = rgb_png(2, 2, [10, 20, 30]);
        let rgba = solid_rgba(2, 2, [10, 20, 30]);
        let mut checks = Vec::new();
        let diff = verify_reference(&frame(&rgba, 2, 2), &png, "ref.png", &mut checks).unwrap();
        assert!(diff.is_none());
        assert_eq!(checks.len(), 1);
        assert!(checks[0].passed);
        assert_eq!(checks[0].actual, "exact match");
    }

    #[test]
    fn reference_mismatch_returns_diff() {
        let png = rgb_png(2, 2, [10, 20, 30]);
        let rgba = solid_rgba(2, 2, [255, 0, 0]);
        let mut checks = Vec::new();
        let diff = verify_reference(&frame(&rgba, 2, 2), &png, "ref.png", &mut checks).unwrap();
        let png = diff.expect("mismatch must produce a diff image");
        assert!(!checks[0].passed);
        assert!(checks[0].actual.contains("4 differing pixels"));
        let (w, h, _) = media::decode_png_rgb(&png).unwrap();
        assert_eq!(
            (w, h),
            (6, 2),
            "diff must be actual|reference|diff side by side"
        );
    }

    #[test]
    fn reference_size_mismatch_fails() {
        let png = rgb_png(2, 2, [10, 20, 30]);
        let rgba = solid_rgba(1, 1, [10, 20, 30]);
        let mut checks = Vec::new();
        let diff = verify_reference(&frame(&rgba, 1, 1), &png, "ref.png", &mut checks).unwrap();
        assert!(diff.is_none());
        assert!(!checks[0].passed);
        assert!(checks[0].actual.contains("reference is 2x2"));
    }

    #[test]
    fn validate_accepts_default_spec() {
        assert!(VerifySpec::default().validate().is_ok());
    }

    #[test]
    fn validate_rejects_non_hex() {
        let spec = VerifySpec {
            memory: vec![MemoryEntry {
                address: "zz".to_string(),
                value: "00".to_string(),
            }],
            ..VerifySpec::default()
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn validate_rejects_out_of_range() {
        let spec = VerifySpec {
            memory: vec![MemoryEntry {
                address: "0x10000".to_string(),
                value: "0xFF".to_string(),
            }],
            ..VerifySpec::default()
        };
        assert!(spec.validate().is_err());
        let spec = VerifySpec {
            memory: vec![MemoryEntry {
                address: "0xC000".to_string(),
                value: "0x100".to_string(),
            }],
            ..VerifySpec::default()
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn memory_comparison_validates_addresses_and_reports_values() {
        let spec = VerifySpec {
            memory_comparisons: vec![MemoryComparison {
                actual_address: "0xFF80".to_string(),
                expected_address: "0xFF81".to_string(),
                name: Some("rom_result".to_string()),
                when: None,
            }],
            ..VerifySpec::default()
        };
        assert!(spec.validate().is_ok());

        let mut bus = GbcMemoryBus::new();
        bus.write(0xFF80, 0x12);
        bus.write(0xFF81, 0x34);
        let mut checks = Vec::new();
        spec.verify_memory(&bus, &mut checks).unwrap();

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "rom_result");
        assert_eq!(checks[0].actual, "$12");
        assert_eq!(checks[0].expected, "$34");
        assert!(!checks[0].passed);

        let conditional = VerifySpec {
            memory_comparisons: vec![MemoryComparison {
                actual_address: "0xFF80".to_string(),
                expected_address: "0xFF81".to_string(),
                name: None,
                when: Some(MemoryCondition {
                    address: "0xFF82".to_string(),
                    value: Some("0xFF".to_string()),
                    not_value: None,
                }),
            }],
            ..VerifySpec::default()
        };
        checks.clear();
        conditional.verify_memory(&bus, &mut checks).unwrap();
        assert!(checks.is_empty());

        let invalid = VerifySpec {
            memory_comparisons: vec![MemoryComparison {
                actual_address: "0x10000".to_string(),
                expected_address: "0xFF81".to_string(),
                name: None,
                when: None,
            }],
            ..VerifySpec::default()
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn serial_hash_matches_only_complete_output() {
        let complete = b"Passed";
        let spec = VerifySpec {
            serial: Some(SerialVerify {
                hash: Some(Hash::Plain(format!("{:08X}", crc32(complete)))),
                ..SerialVerify::default()
            }),
            ..VerifySpec::default()
        };
        assert!(!spec.serial_hash_matches(b"Pass"));
        assert!(spec.serial_hash_matches(complete));
    }
}
