use std::path::{Path, PathBuf};

use super::{
    error::RomTestError,
    events::{ControllerPad, PadState, RomAssertion, RomEvent, RomEventKind},
    harness::{CaseHarness, drive_case},
    manifest::{RomCase, RomCategory, RomManifest, default_manifest_path, load_default_manifest},
};

#[test]
fn parse_manifest_with_hex_values() {
    let mut manifest = serde_saphyr::from_str::<RomManifest>(
        r#"
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: nes-test-roms/other/nestest.nes
    perf: true
    options: ["--mmc3-irq-variant", "nec", "--submapper", "4"]
    expected_audio:
      sample_rate: 192000
      samples: 287270
      hash: "0x34BB3FFDF962043D"
    events:
      - { frame: 15, action: check_memory, address: "0x0301", value: "0x01" }
      - { frame: 15, action: check_screen, hash: "0x464033EFDAB11D8E" }
      - { frame: 15, action: standard_controller, pad: pad1, button: nes.control.start, state: pressed }
"#,
    )
    .expect("manifest should parse");
    manifest.resolve_paths(&default_manifest_path());
    manifest.validate().expect("manifest should validate");
    assert!(manifest.case("cpu.nestest").unwrap().perf);
    assert_eq!(
        manifest.case("cpu.nestest").unwrap().options,
        vec![
            "--mmc3-irq-variant".to_string(),
            "nec".to_string(),
            "--submapper".to_string(),
            "4".to_string()
        ]
    );
}

#[test]
fn parse_manifest_with_generic_assertions() {
    let mut manifest = serde_saphyr::from_str::<RomManifest>(
        r#"
cases:
  - id: mapper.generic_assert
    category: mapper
    description: Generic assertion parsing regression.
    rom: nes-test-roms/mmc3_test/6-MMC6.nes
    events:
      - { frame: 15, action: assert, kind: memory, address: "0x6000", value: "0x00", open_bus: true }
      - { frame: 15, action: assert, kind: screen, hash: "0x464033EFDAB11D8E" }
"#,
    )
    .expect("manifest should parse");
    manifest.resolve_paths(&default_manifest_path());
    manifest.validate().expect("manifest should validate");

    match &manifest.case("mapper.generic_assert").unwrap().events[0].kind {
        RomEventKind::Assert {
            assertion:
                RomAssertion::Memory {
                    address,
                    value,
                    open_bus,
                },
        } => {
            assert_eq!(*address, 0x6000);
            assert_eq!(*value, 0x00);
            assert!(*open_bus);
        }
        other => panic!("unexpected event kind: {other:?}"),
    }
}

#[test]
fn default_manifest_contains_perf_cases() {
    let manifest = load_default_manifest().expect("default manifest should load");
    assert!(manifest.case("cpu.nestest").is_some());
    assert!(manifest.case("apu.len_ctr").is_some());
    assert!(manifest.case("ppu.vbl_nmi").is_some());
}

#[test]
fn resolve_manifest_paths_relative_to_manifest_file() {
    let mut manifest = serde_saphyr::from_str::<RomManifest>(
        r#"
rom_root: fixtures/roms
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: nes-test-roms/other/nestest.nes
    events:
      - { frame: 1, action: check_screen, hash: "0x1" }
"#,
    )
    .expect("manifest should parse");

    manifest.resolve_paths(Path::new("/tmp/config/rom_tests.yaml"));

    assert_eq!(
        manifest
            .case("cpu.nestest")
            .unwrap()
            .resolved_rom_path()
            .expect("path should resolve"),
        Path::new("/tmp/config")
            .join("fixtures/roms")
            .join("nes-test-roms/other/nestest.nes")
    );
}

#[test]
fn drive_case_dispatches_frame_zero_events() {
    struct Harness {
        frame_counter: u64,
        events: Vec<String>,
    }

    impl CaseHarness for Harness {
        fn run_frame(&mut self) -> Result<(), RomTestError> {
            self.frame_counter += 1;
            Ok(())
        }

        fn frame_counter(&self) -> u64 {
            self.frame_counter
        }

        fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError> {
            match assertion {
                RomAssertion::Screen { .. } => self.events.push(format!("check@{frame}")),
                RomAssertion::Memory { open_bus, .. } if *open_bus => {
                    self.events.push(format!("cart@{frame}"))
                }
                RomAssertion::Memory { .. } => self.events.push(format!("ram@{frame}")),
            }
            Ok(())
        }

        fn on_reset(&mut self) -> Result<(), RomTestError> {
            self.events.push(format!("reset@{}", self.frame_counter));
            Ok(())
        }

        fn on_standard_controller(
            &mut self,
            _pad: ControllerPad,
            _button: String,
            _state: PadState,
        ) -> Result<(), RomTestError> {
            self.events
                .push(format!("controller@{}", self.frame_counter));
            Ok(())
        }
    }

    let case = RomCase {
        id: "frame-zero".to_string(),
        category: RomCategory::Cpu,
        description: "Frame-zero dispatch regression.".to_string(),
        rom: "nes-test-roms/other/nestest.nes".to_string(),
        perf: false,
        options: Vec::new(),
        events: vec![
            RomEvent {
                frame: 0,
                kind: RomEventKind::Reset,
            },
            RomEvent {
                frame: 0,
                kind: RomEventKind::StandardController {
                    pad: ControllerPad::Pad1,
                    button: "nes.control.start".to_string(),
                    state: PadState::Pressed,
                },
            },
            RomEvent {
                frame: 0,
                kind: RomEventKind::StandardController {
                    pad: ControllerPad::Pad2,
                    button: "famicom.microphone".to_string(),
                    state: PadState::Pressed,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckMemory {
                    address: 0x0301,
                    value: 0x01,
                    open_bus: false,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckMemory {
                    address: 0x6000,
                    value: 0x00,
                    open_bus: true,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckMemory {
                    address: 0x2000,
                    value: 0x00,
                    open_bus: false,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckScreen { hash: 1 },
            },
        ],
        expected_audio: None,
        ci: true,
        resolved_rom_path: PathBuf::new(),
    };
    let mut harness = Harness {
        frame_counter: 0,
        events: Vec::new(),
    };

    let totals = drive_case(&case, &mut harness).expect("case should run");

    assert_eq!(totals.frames, 1);
    assert_eq!(
        harness.events,
        vec![
            "reset@0".to_string(),
            "controller@0".to_string(),
            "controller@0".to_string(),
            "ram@1".to_string(),
            "cart@1".to_string(),
            "ram@1".to_string(),
            "check@1".to_string()
        ]
    );
}

#[test]
fn rom_case_builds_core_options() {
    let case = RomCase {
        id: "mapper.option".to_string(),
        category: RomCategory::Mapper,
        description: "Option regression.".to_string(),
        rom: "mapper/option.nes".to_string(),
        perf: false,
        options: vec![
            "--mmc3-irq-variant".to_string(),
            "nec".to_string(),
            "--submapper".to_string(),
            "4".to_string(),
        ],
        events: vec![RomEvent {
            frame: 1,
            kind: RomEventKind::CheckScreen { hash: 1 },
        }],
        expected_audio: None,
        ci: true,
        resolved_rom_path: PathBuf::new(),
    };

    assert_eq!(
        case.options,
        vec![
            "--mmc3-irq-variant".to_string(),
            "nec".to_string(),
            "--submapper".to_string(),
            "4".to_string()
        ]
    );
}

#[test]
fn rom_case_passes_submapper_range_check_to_load_time() {
    // Option validity belongs to the factory/core that defines the
    // flags: the manifest passes options through untouched, and the
    // out-of-range value fails loudly at load through `CartridgeData`
    // validation (covered by `nes_core` unit tests).
    let mut manifest = serde_saphyr::from_str::<RomManifest>(
        r#"
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: nes-test-roms/other/nestest.nes
    options: ["--submapper", "16"]
    events:
      - { frame: 1, action: check_screen, hash: "0x1" }
"#,
    )
    .expect("manifest should parse");
    manifest.resolve_paths(&default_manifest_path());

    manifest.validate().expect("options pass through");
}
