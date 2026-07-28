# GBC Test ROMs

## Upstreams

| Directory | Source | License |
|---|---|---|
| `retrio_gb-test-roms/` | https://github.com/retrio/gb-test-roms | See individual ROMs |

## Usage

Test ROMs are executed via Rust integration tests in `rom_test/`.
Each test loads the ROM, runs it through the emulator, and checks
for expected output (e.g., serial port messages, memory checksums).
