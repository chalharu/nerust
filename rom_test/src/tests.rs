use super::error::RomTestError;
use super::events::{
    ButtonCode, ControllerPad, MemoryAssertionSpace, PadState, RomAssertion, RomEvent, RomEventKind,
};
use super::harness::{CaseHarness, drive_case};
use super::manifest::{
    RomCase, RomCategory, RomManifest, apply_case_rom_overrides, default_manifest_path,
    load_default_manifest,
};
use nerust_nes_core::core_options::Mmc3IrqVariant;
use std::path::{Path, PathBuf};

#[test]
fn parse_manifest_with_hex_values() {
    let mut manifest = serde_yaml::from_str::<RomManifest>(
        r#"
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: nes-test-roms/other/nestest.nes
    perf: true
    sub_mapper_type: 4
    mmc3_irq_variant: nec
    expected_audio:
      sample_rate: 192000
      samples: 287270
      hash: "0x34BB3FFDF962043D"
    events:
      - { frame: 15, action: check_work_ram, address: "0x0301", value: "0x01" }
      - { frame: 15, action: check_screen, hash: "0x464033EFDAB11D8E" }
      - { frame: 15, action: standard_controller, pad: pad1, button: START, state: pressed }
"#,
    )
    .expect("manifest should parse");
    manifest.resolve_paths(&default_manifest_path());
    manifest.validate().expect("manifest should validate");
    assert!(manifest.case("cpu.nestest").unwrap().perf);
    assert_eq!(
        manifest.case("cpu.nestest").unwrap().sub_mapper_type,
        Some(4)
    );
    assert_eq!(
        manifest.case("cpu.nestest").unwrap().mmc3_irq_variant,
        Some(Mmc3IrqVariant::Nec)
    );
}

#[test]
fn parse_manifest_with_generic_assertions() {
    let mut manifest = serde_yaml::from_str::<RomManifest>(
        r#"
cases:
  - id: mapper.generic_assert
    category: mapper
    description: Generic assertion parsing regression.
    rom: nes-test-roms/mmc3_test/6-MMC6.nes
    events:
      - { frame: 15, action: assert, kind: memory, space: cartridge_ram, address: "0x6000", value: "0x00", open_bus: true }
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
                    space,
                    address,
                    value,
                    open_bus,
                },
        } => {
            assert_eq!(*space, MemoryAssertionSpace::CartridgeRam);
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
    let mut manifest = serde_yaml::from_str::<RomManifest>(
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
        fn run_frame(&mut self) -> u64 {
            self.frame_counter += 1;
            1
        }

        fn frame_counter(&self) -> u64 {
            self.frame_counter
        }

        fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError> {
            match assertion {
                RomAssertion::Screen { .. } => self.events.push(format!("check@{frame}")),
                RomAssertion::Memory { space, .. } => match space {
                    MemoryAssertionSpace::WorkRam => self.events.push(format!("ram@{frame}")),
                    MemoryAssertionSpace::CartridgeRam => self.events.push(format!("cart@{frame}")),
                    MemoryAssertionSpace::PpuVram => self.events.push(format!("ppu@{frame}")),
                },
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
            _button: ButtonCode,
            _state: PadState,
        ) -> Result<(), RomTestError> {
            self.events
                .push(format!("controller@{}", self.frame_counter));
            Ok(())
        }

        fn on_microphone(&mut self, _state: PadState) -> Result<(), RomTestError> {
            self.events
                .push(format!("microphone@{}", self.frame_counter));
            Ok(())
        }
    }

    let case = RomCase {
        id: "frame-zero".to_string(),
        category: RomCategory::Cpu,
        description: "Frame-zero dispatch regression.".to_string(),
        rom: "nes-test-roms/other/nestest.nes".to_string(),
        perf: false,
        sub_mapper_type: None,
        mmc3_irq_variant: None,
        events: vec![
            RomEvent {
                frame: 0,
                kind: RomEventKind::Reset,
            },
            RomEvent {
                frame: 0,
                kind: RomEventKind::StandardController {
                    pad: ControllerPad::Pad1,
                    button: ButtonCode::START,
                    state: PadState::Pressed,
                },
            },
            RomEvent {
                frame: 0,
                kind: RomEventKind::Microphone {
                    state: PadState::Pressed,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckWorkRam {
                    address: 0x0301,
                    value: 0x01,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckCartridgeRam {
                    address: 0x6000,
                    value: 0x00,
                    open_bus: false,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckPpuVram {
                    address: 0x2000,
                    value: 0x00,
                },
            },
            RomEvent {
                frame: 1,
                kind: RomEventKind::CheckScreen { hash: 1 },
            },
        ],
        expected_audio: None,
        resolved_rom_path: PathBuf::new(),
    };
    let mut harness = Harness {
        frame_counter: 0,
        events: Vec::new(),
    };

    let totals = drive_case(&case, &mut harness).expect("case should run");

    assert_eq!(totals.frames, 1);
    assert_eq!(totals.steps, 1);
    assert_eq!(
        harness.events,
        vec![
            "reset@0".to_string(),
            "controller@0".to_string(),
            "microphone@0".to_string(),
            "ram@1".to_string(),
            "cart@1".to_string(),
            "ppu@1".to_string(),
            "check@1".to_string()
        ]
    );
}

#[test]
fn check_work_ram_rejects_addresses_outside_cpu_work_ram() {
    let event = RomEvent {
        frame: 0,
        kind: RomEventKind::CheckWorkRam {
            address: 0x2000,
            value: 0,
        },
    };

    assert!(matches!(
        event.validate("mapper.34_test_src.34_test_1"),
        Err(RomTestError::InvalidManifest(message))
            if message.contains("check_work_ram outside CPU work RAM")
    ));
}

#[test]
fn check_cartridge_ram_rejects_addresses_outside_cartridge_ram() {
    let event = RomEvent {
        frame: 0,
        kind: RomEventKind::CheckCartridgeRam {
            address: 0x5FFF,
            value: 0,
            open_bus: false,
        },
    };

    assert!(matches!(
        event.validate("mapper.mmc3_test.1-clocking"),
        Err(RomTestError::InvalidManifest(message))
            if message.contains("check_cartridge_ram outside cartridge RAM")
    ));
}

#[test]
fn check_ppu_vram_rejects_addresses_outside_nametable_space() {
    let event = RomEvent {
        frame: 0,
        kind: RomEventKind::CheckPpuVram {
            address: 0x1FFF,
            value: 0,
        },
    };

    assert!(matches!(
        event.validate("mapper.mmc3bigchrram"),
        Err(RomTestError::InvalidManifest(message))
            if message.contains("check_ppu_vram outside PPU nametable/palette space")
    ));
}

#[test]
fn generic_memory_assert_rejects_open_bus_outside_cartridge_ram() {
    let event = RomEvent {
        frame: 0,
        kind: RomEventKind::Assert {
            assertion: RomAssertion::Memory {
                space: MemoryAssertionSpace::WorkRam,
                address: 0x0000,
                value: 0,
                open_bus: true,
            },
        },
    };

    assert!(matches!(
        event.validate("mapper.generic_assert"),
        Err(RomTestError::InvalidManifest(message))
            if message.contains("open_bus with a non-cartridge memory assertion")
    ));
}

#[test]
fn rom_case_builds_core_options() {
    let case = RomCase {
        id: "mapper.option".to_string(),
        category: RomCategory::Mapper,
        description: "Option regression.".to_string(),
        rom: "mapper/option.nes".to_string(),
        perf: false,
        sub_mapper_type: Some(4),
        mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
        events: vec![RomEvent {
            frame: 1,
            kind: RomEventKind::CheckScreen { hash: 1 },
        }],
        expected_audio: None,
        resolved_rom_path: PathBuf::new(),
    };

    assert_eq!(
        case.core_options().mmc3_irq_variant,
        Some(Mmc3IrqVariant::Nec)
    );
    assert_eq!(case.sub_mapper_type, Some(4));
}

#[test]
fn rom_case_rejects_submapper_values_outside_nes20_range() {
    let mut manifest = serde_yaml::from_str::<RomManifest>(
        r#"
cases:
  - id: cpu.nestest
    category: cpu
    description: Best first-pass CPU validation ROM.
    rom: nes-test-roms/other/nestest.nes
    sub_mapper_type: 16
    events:
      - { frame: 1, action: check_screen, hash: "0x1" }
"#,
    )
    .expect("manifest should parse");
    manifest.resolve_paths(&default_manifest_path());

    assert!(matches!(
        manifest.validate(),
        Err(RomTestError::InvalidManifest(message))
            if message.contains("sub_mapper_type")
    ));
}

#[test]
fn submapper_override_promotes_rom_header_in_memory() {
    let case = RomCase {
        id: "mapper.override".to_string(),
        category: RomCategory::Mapper,
        description: "Override regression.".to_string(),
        rom: "mapper/override.nes".to_string(),
        perf: false,
        sub_mapper_type: Some(1),
        mmc3_irq_variant: None,
        events: vec![RomEvent {
            frame: 1,
            kind: RomEventKind::CheckScreen { hash: 1 },
        }],
        expected_audio: None,
        resolved_rom_path: PathBuf::new(),
    };
    let rom_bytes = vec![
        0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];

    let overridden = apply_case_rom_overrides(&case, rom_bytes).expect("override should work");

    assert_eq!(overridden[7], 0x08);
    assert_eq!(overridden[8], 0x10);
}

#[test]
fn submapper_override_clears_ines_prg_ram_bits_when_promoting_to_nes20() {
    let case = RomCase {
        id: "mapper.override".to_string(),
        category: RomCategory::Mapper,
        description: "Override regression.".to_string(),
        rom: "mapper/override.nes".to_string(),
        perf: false,
        sub_mapper_type: Some(1),
        mmc3_irq_variant: None,
        events: vec![RomEvent {
            frame: 1,
            kind: RomEventKind::CheckScreen { hash: 1 },
        }],
        expected_audio: None,
        resolved_rom_path: PathBuf::new(),
    };
    let rom_bytes = vec![
        0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x41, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];

    let overridden = apply_case_rom_overrides(&case, rom_bytes).expect("override should work");

    assert_eq!(overridden[7], 0x08);
    assert_eq!(overridden[8], 0x10);
}
