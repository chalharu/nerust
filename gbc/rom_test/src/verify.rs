use serde::{Deserialize, Serialize};

use nerust_gbc_core::memory::GbcMemoryBus;

use super::{error::RomTestError, media};

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
}

impl VerifySpec {
    /// Validate that all declared hex values parse and fit their targets.
    /// Called at manifest load time so configuration errors fail fast
    /// instead of surfacing per cell at run time.
    pub fn validate(&self) -> Result<(), RomTestError> {
        if let Some(serial) = &self.serial {
            if let Some(hash) = &serial.hash {
                parse_hex_bytes(hash.value())?;
            }
            if let Some(suffix) = &serial.suffix {
                parse_hex_bytes(suffix)?;
            }
        }
        if let Some(frame) = &self.frame {
            parse_hex_bytes(frame.hash.value())?;
        }
        for entry in &self.memory {
            let address = parse_hex(&entry.address)?;
            if address > u16::MAX as u64 {
                return Err(RomTestError::InvalidManifest(format!(
                    "invalid memory address: {}",
                    entry.address
                )));
            }
            let value = parse_hex(&entry.value)?;
            if value > u8::MAX as u64 {
                return Err(RomTestError::InvalidManifest(format!(
                    "invalid memory value: {}",
                    entry.value
                )));
            }
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
            let actual = bus.read(addr);
            checks.push(CheckResult {
                name: format!("memory@${:04X}", addr),
                expected: format!("${:02X}", expected),
                actual: format!("${:02X}", actual),
                passed: actual == expected,
            });
        }
        Ok(())
    }
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
    for px in frame.rgba.chunks_exact(4) {
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
        .chunks_exact(3)
        .zip(ref_rgb.chunks_exact(3))
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

fn parse_hex(s: &str) -> Result<u64, RomTestError> {
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
}
