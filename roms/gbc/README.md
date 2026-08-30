# GBC Test ROMs

## Upstreams

| Directory | Source | License |
|---|---|---|
| `retrio_gb-test-roms/` | https://github.com/retrio/gb-test-roms | See individual ROMs |
| `mattcurrie_cgb-acid2/` | https://github.com/mattcurrie/cgb-acid2 | MIT |
| `mattcurrie_dmg-acid2/` | https://github.com/mattcurrie/dmg-acid2 | MIT |
| `mattcurrie_mealybug-tearoom-tests/` | https://github.com/mattcurrie/mealybug-tearoom-tests | MIT |
| `Gekkio_mooneye-test-suite/` | https://github.com/Gekkio/mooneye-test-suite | MIT |
| `aappleby_gbmicrotest/` | https://github.com/aappleby/gbmicrotest | MIT |
| `aaaaaa123456789_rtc3test/` | https://github.com/aaaaaa123456789/rtc3test | Unlicense |
| `c-sp_age-test-roms/` | https://github.com/c-sp/age-test-roms | MIT |
| `EricKirschenmann_MBC3-Tester-gb/` | https://github.com/EricKirschenmann/MBC3-Tester-gb | No license file; original source attribution in upstream README |

Each upstream is imported as a squashed Git subtree under its `repo/` directory.

## Build and artifact provenance

- GBMicrotest includes 513 prebuilt mapper-free ROMs under `repo/bin/`. Rebuild
	them with WLA-DX and `repo/build.sh`.
- AGE test ROMs require RGBDS. Run `make -C repo`; the 50 ROMs and 11 reference
	PNGs used by the matrix are copied from `repo/build/` to `artifacts/`.
- rtc3test v004 requires an older RGBDS syntax that is incompatible with RGBDS
	0.9.4. `release/rtc3test.gb` is the upstream v004 release asset with SHA-256
	`a271013fb37ea1d927b854798401320b220d57abb3ca77f3d318ceb8a9def30d`.
- MBC3 Tester uses the upstream v1.0 release asset, SHA-256
	`6cec9072f611d0e1f99e276255115ab7672553e0250719a89fd0c8a85a7b4149`.
	The asset is 4 KiB shorter than its declared 4 MiB ROM size. The matrix uses
	`release/mbctest-padded.gb`, zero-extended to 4 MiB, with SHA-256
	`134977aa5d5dd6f9f5533fcff72b8dedb81d68571607fa1127026e1b8bec6ff5`.

## Pass criteria

- GBMicrotest passes when HRAM address `0xFF82` becomes `0x01`; `0xFF` is an
	explicit failure. The current matrix passes 324 of 513 tests.
- AGE non-visual tests pass with the documented register signature
	`B,C,D,E,H,L = 3,5,8,13,21,34`. Visual tests are compared against upstream
	PNGs using the README palette rules. The current matrix passes 8 of 96 cells.
	The three upstream `_in-progress` ROMs are not registered.
- rtc3test passes when the Basic, Range, and Sub-second result screens exactly
	match the captured all-PASS screens. Their displayed timing values satisfy
	the tolerances documented in upstream `tests.md`.
- MBC3 Tester passes when all 255 bank cells contain its pass tile. The CGB
	compatibility-mode cell passes; the DMG rendering cell remains a known PPU
	failure.

## Usage

Test ROMs are executed via Rust integration tests in `rom_test/`.
Each test loads the ROM, runs it through the emulator, and checks
for expected output (e.g., serial port messages, memory checksums).
