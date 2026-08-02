# GBC Test ROMs

## Upstreams

| Directory | Source | License |
|---|---|---|
| `retrio_gb-test-roms/` | https://github.com/retrio/gb-test-roms | See individual ROMs |
| `mattcurrie_cgb-acid2/` | https://github.com/mattcurrie/cgb-acid2 | MIT |
| `mattcurrie_dmg-acid2/` | https://github.com/mattcurrie/dmg-acid2 | MIT |
| `mattcurrie_mealybug-tearoom-tests/` | https://github.com/mattcurrie/mealybug-tearoom-tests | MIT |
| `Gekkio_mooneye-test-suite/` | https://github.com/Gekkio/mooneye-test-suite | MIT |

## Usage

Test ROMs are executed via Rust integration tests in `rom_test/`.
Each test loads the ROM, runs it through the emulator, and checks
for expected output (e.g., serial port messages, memory checksums).
