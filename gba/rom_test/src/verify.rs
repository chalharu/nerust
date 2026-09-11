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
    /// Log-driven multi-test verification (mgba-emu/suite): branch one
    /// ROM's `PASS:`/`FAIL:` debug-log lines into per-subtest checks.
    #[serde(default)]
    pub suite_log: Option<SuiteLogVerify>,
}

/// Scope for [`verify_suite_log`]: the marker lines a guest log emits
/// around each test group, plus the per-result line prefixes. All
/// strings are manifest configuration (no ROM-specific knowledge in
/// the engine); e.g. mgba-emu/suite uses `BEGIN:`/`END:` with
/// `PASS: `/`FAIL: ` lines from its `runSuite`/`doResult` helpers.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SuiteLogVerify {
    pub begin: String,
    pub end: String,
    pub pass_prefix: String,
    pub fail_prefix: String,
    /// Optional SRAM failure-detail enrichment (see
    /// [`enrich_suite_log_checks`]). Absent = checks carry plain
    /// pass/fail without guest-provided values.
    #[serde(default)]
    pub sram: Option<SramEnrichment>,
}

/// Marker shapes for [`enrich_suite_log_checks`]: which SRAM lines are
/// failure details, which open a new test scope, and how a detail's
/// preface key is separated from its values. All manifest
/// configuration; e.g. mgba-emu/suite details look like
/// `<preface>: Got 0x... vs 0x...: FAIL` under `<Suite> test: <name>`
/// headers.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SramEnrichment {
    pub fail_suffix: String,
    pub header_infix: String,
    pub detail_infix: String,
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
        self.memory.is_empty()
            && self.registers.is_empty()
            && self.frame_pixels.is_empty()
            && self.suite_log.is_none()
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
        if let Some(suite_log) = &self.suite_log {
            // Empty markers would match every line (or nothing); require
            // explicit non-empty configuration.
            if suite_log.begin.is_empty()
                || suite_log.end.is_empty()
                || suite_log.pass_prefix.is_empty()
                || suite_log.fail_prefix.is_empty()
            {
                return Err(RomTestError::InvalidManifest(
                    "suite_log markers must all be non-empty".to_string(),
                ));
            }
            if let Some(markers) = &suite_log.sram
                && (markers.fail_suffix.is_empty()
                    || markers.header_infix.is_empty()
                    || markers.detail_infix.is_empty())
            {
                return Err(RomTestError::InvalidManifest(
                    "suite_log sram markers must all be non-empty".to_string(),
                ));
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

/// Attach guest-provided failure details from a memory log (e.g. a
/// suite's SRAM `savprintf` area) to failing log checks.
///
/// The memory text holds one detail line per failure, in the same
/// emission order as the log's fail lines, scoped by per-test headers.
/// Details are zipped positionally within each scope; a detail carrying
/// an explicit preface key (text before `markers.detail_infix`) is only
/// attached when the check name ends with that preface, otherwise the
/// check keeps its plain actual. Headers are lines containing
/// `markers.header_infix`, with the scope name after the infix; suites
/// without headers share one global scope.
pub fn enrich_suite_log_checks(
    checks: &mut [CheckResult],
    sram_text: &str,
    markers: &SramEnrichment,
) {
    // scope -> details in emission order. `String::new()` is the global
    // scope for suites without headers.
    let mut scopes: std::collections::BTreeMap<String, Vec<(Option<String>, String)>> =
        std::collections::BTreeMap::new();
    let mut current = String::new();
    for line in sram_text.split('\n') {
        let line = line.trim_end_matches('\0').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(body) = line.strip_suffix(markers.fail_suffix.as_str()) {
            let key = body
                .split_once(markers.detail_infix.as_str())
                .map(|(key, _)| key.trim().to_string());
            let key = key.filter(|key| !key.is_empty());
            scopes
                .entry(current.clone())
                .or_default()
                .push((key, line.to_string()));
        } else if let Some((_, name)) = line.split_once(markers.header_infix.as_str()) {
            current = name.trim().to_string();
        }
    }
    if scopes.is_empty() {
        return;
    }
    let mut consumed: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for check in checks.iter_mut().filter(|check| !check.passed) {
        // Longest header scope that prefixes the check name ("ROM load"
        // for "ROM load U16 (unaligned)"); fall back to global scope.
        let mut scope = String::new();
        for name in scopes.keys() {
            if !name.is_empty()
                && (check.name == *name || check.name.starts_with(&format!("{name} ")))
                && name.len() > scope.len()
            {
                scope = name.clone();
            }
        }
        let Some(details) = scopes.get(&scope) else {
            continue;
        };
        let used = consumed.entry(scope.clone()).or_insert(0);
        // Prefer the next detail whose preface key matches; otherwise take
        // the next detail positionally (emission order is guaranteed).
        let mut pick = None;
        for (index, (key, _)) in details.iter().enumerate().skip(*used) {
            if key.as_ref().is_some_and(|key| check.name.ends_with(key)) {
                pick = Some(index);
                break;
            }
        }
        let pick = pick.unwrap_or(*used);
        if let Some((_, full)) = details.get(pick) {
            check.actual = full.clone();
        }
        *used = (*used).max(pick + 1);
    }
}

pub struct FramePixels<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

/// Branch one ROM's guest-log lines into per-subtest [`CheckResult`]s.
///
/// Only lines between the first line containing `spec.begin` and the first
/// later line containing `spec.end` are scored; lines starting with
/// `spec.pass_prefix` pass a check named after the prefix, lines starting
/// with `spec.fail_prefix` fail it. Missing markers produce failing scope
/// checks (a ROM that never reaches `end` timed out or crashed) while
/// still reporting whatever subtests were observed.
pub fn verify_suite_log(
    logs: &[nerust_gba_core::memory::MgbaDebugLog],
    spec: &SuiteLogVerify,
) -> Vec<CheckResult> {
    let mut checks = Vec::new();
    let begin = logs.iter().position(|log| log.text.contains(&spec.begin));
    let Some(begin) = begin else {
        return vec![CheckResult {
            name: "suite log begin".into(),
            expected: spec.begin.clone(),
            actual: "marker never logged".into(),
            passed: false,
        }];
    };
    let mut window = Vec::new();
    for log in &logs[begin..] {
        let done = log.text.contains(&spec.end);
        window.push(log.text.as_str());
        if done {
            break;
        }
    }
    let finished = window.iter().any(|line| line.contains(&spec.end));
    for line in window {
        if let Some(rest) = line.strip_prefix(spec.pass_prefix.as_str()) {
            checks.push(CheckResult {
                name: rest.to_string(),
                expected: "pass".into(),
                actual: "PASS".into(),
                passed: true,
            });
        } else if let Some(rest) = line.strip_prefix(spec.fail_prefix.as_str()) {
            checks.push(CheckResult {
                name: rest.to_string(),
                expected: "pass".into(),
                actual: "FAIL".into(),
                passed: false,
            });
        }
    }
    if finished {
        if checks.is_empty() {
            checks.push(CheckResult {
                name: "suite log results".into(),
                expected: "at least one PASS/FAIL line".into(),
                actual: "none in scope".into(),
                passed: false,
            });
        }
    } else {
        checks.push(CheckResult {
            name: "suite log end".into(),
            expected: spec.end.clone(),
            actual: "marker never logged".into(),
            passed: false,
        });
    }
    checks
}

pub fn verify_reference(
    frame: &FramePixels<'_>,
    ref_png: &[u8],
    expected_label: &str,
    checks: &mut Vec<CheckResult>,
) -> Result<Option<Vec<u8>>, RomTestError> {
    let (rw, rh, ref_rgb) = crate::media::decode_png_rgb(ref_png)?;
    let width = frame.width;
    let height = frame.height;
    let expected = expected_label.to_string();

    // 実機写真 (4000x3000等) はpixel比較できないが、未検証をpassにはしない。
    if rw != width || rh != height {
        checks.push(CheckResult {
            name: "reference dimensions".to_string(),
            expected: format!("{}x{}", width, height),
            actual: format!("{}x{}", rw, rh),
            passed: false,
        });
        return Ok(None);
    }

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

    // BGR555 tolerance: GBAは15bit(5bit/channel)なので、8bit展開差(<<3 vs <<3|>>2)は
    // 同一BGR555に丸めれば一致する。参照PNGが単純<<3で生成されていても
    // エミュレータのハードウェア正確な展開と一致とみなす。
    let to_bgr555 = |rgb: &[u8; 3]| {
        let r = u16::from(rgb[0] >> 3);
        let g = u16::from(rgb[1] >> 3);
        let b = u16::from(rgb[2] >> 3);
        (b << 10) | (g << 5) | r
    };
    let frame_bgr: Vec<u16> = frame_rgb.as_chunks::<3>().0.iter().map(to_bgr555).collect();
    let ref_bgr: Vec<u16> = ref_rgb.as_chunks::<3>().0.iter().map(to_bgr555).collect();
    if frame_bgr == ref_bgr {
        checks.push(CheckResult {
            name: "reference".to_string(),
            expected,
            actual: "BGR555 match (8bit expansion tolerant)".to_string(),
            passed: true,
        });
        return Ok(None);
    }

    let mut diff_count = 0usize;
    let mut bgr_diff_count = 0usize;
    let mut first = None;
    for (i, ((a, b), (fa, fb))) in frame_rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(ref_rgb.as_chunks::<3>().0.iter())
        .zip(frame_bgr.iter().zip(ref_bgr.iter()))
        .enumerate()
    {
        if a != b {
            if first.is_none() {
                first = Some((i % width as usize, i / width as usize));
            }
            diff_count += 1;
        }
        if fa != fb {
            bgr_diff_count += 1;
        }
    }
    let (fx, fy) = first.unwrap_or((0, 0));
    // Report both counts: strict RGB (8-bit expansion sensitive) and BGR555
    // (real color differences, e.g. wrong texels). Old-emu references using
    // v<<3 expansion inflate the strict count even when geometry matches.
    let actual = format!(
        "{} differing pixels ({} after BGR555 rounding), first at ({},{})",
        diff_count, bgr_diff_count, fx, fy
    );
    checks.push(CheckResult {
        name: "reference".to_string(),
        expected,
        actual,
        passed: false,
    });

    let diff = crate::media::compose_diff_image(&frame_rgb, &ref_rgb, width, height);
    let png = crate::media::encode_rgba_png(width * 3, height, &diff)?;
    Ok(Some(png))
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let mut digest = crc.digest();
    digest.update(data);
    digest.finalize()
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

    #[test]
    fn suite_log_branches_and_scopes() {
        use nerust_gba_core::memory::MgbaDebugLog;
        let logs = [
            "Game Boy Advance Test Suite",
            "BEGIN: Memory tests",
            "PASS: ROM load U8",
            "FAIL: ROM load U16 (unaligned)",
            "END: 1/2",
        ]
        .map(|text| MgbaDebugLog {
            level: 4,
            text: text.into(),
        });
        let spec = SuiteLogVerify {
            begin: "BEGIN: Memory tests".into(),
            end: "END:".into(),
            pass_prefix: "PASS: ".into(),
            fail_prefix: "FAIL: ".into(),
            sram: Some(SramEnrichment {
                fail_suffix: ": FAIL".into(),
                header_infix: " test: ".into(),
                detail_infix: ": Got".into(),
            }),
        };
        let mut checks = verify_suite_log(&logs, &spec);
        assert_eq!(checks.len(), 2);
        enrich_suite_log_checks(
            &mut checks,
            "Memory test: ROM load\nU16 (unaligned): Got 0x00000004 vs 0x00000008: FAIL\n",
            spec.sram.as_ref().unwrap(),
        );
        let fail = checks
            .iter()
            .find(|c| c.name == "ROM load U16 (unaligned)")
            .unwrap();
        assert!(!fail.passed);
        assert!(fail.actual.contains("Got 0x00000004 vs 0x00000008"));
        let pass = checks.iter().find(|c| c.name == "ROM load U8").unwrap();
        assert!(pass.passed);
    }

    #[test]
    fn suite_log_reports_missing_markers() {
        use nerust_gba_core::memory::MgbaDebugLog;
        let logs = [MgbaDebugLog {
            level: 4,
            text: "nothing".into(),
        }];
        let spec = SuiteLogVerify {
            begin: "BEGIN: X".into(),
            end: "END:".into(),
            pass_prefix: "PASS: ".into(),
            fail_prefix: "FAIL: ".into(),
            sram: None,
        };
        let checks = verify_suite_log(&logs, &spec);
        assert_eq!(checks.len(), 1);
        assert!(!checks[0].passed);
    }

    #[test]
    fn rejects_reference_with_incomparable_dimensions() {
        let reference = crate::media::encode_rgba_png(1, 1, &[0, 0, 0, 0xFF]).unwrap();
        let frame = FramePixels {
            rgba: &[0, 0, 0, 0xFF, 0, 0, 0, 0xFF],
            width: 2,
            height: 1,
        };
        let mut checks = Vec::new();

        let diff = verify_reference(&frame, &reference, "reference.png", &mut checks).unwrap();

        assert!(diff.is_none());
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "reference dimensions");
        assert!(!checks[0].passed);
    }
}
