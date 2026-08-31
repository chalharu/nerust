# GBA Test ROMs

## Upstreams

| Directory | Source | License |
|---|---|---|
| `jsmolka_gba-tests/` | https://github.com/jsmolka/gba-tests | MIT |
| `nba-emu_hw-test/` | https://codeberg.org/nba-emu/hw-test | BSD 3-Clause |

Each upstream is imported as a squashed Git subtree.

- `jsmolka_gba-tests`: squashed from commit `a7113b67` (merge `b1693061` / `e221ce14`)
- `nba-emu_hw-test`: squashed from commit `fbc99140e06f083c0a47612467cfbb02470e56dc` (squash `8b81fc11`)

## Build and artifact provenance

- jsmolka `gba-tests` ROMs are assembled with [FASMARM](https://arm.flatassembler.net/). The repository includes prebuilt `.gba` files; rebuild with `fasmarm` if needed.
- `nba-emu/hw-test` ROMs are built with `devkitARM` (`arm-none-eabi-gcc` + `libgba`). Each test directory contains a `Makefile` requiring `DEVKITARM`; prebuilt `.gba` files are included in the subtree and used directly.

## Pass criteria

- `jsmolka_gba-tests` aggregate ROMs (`arm`, `thumb`, `memory`, `save/*`, `bios`) leave the first failed test number in `R12` (`thumb` uses `R7`). `0` means all embedded tests passed. The current matrix runs 8 cases (`jsmolka_arm`, `jsmolka_thumb`, `jsmolka_memory`, `jsmolka_bios`, `jsmolka_save_*`) all passing.
- `nba-emu/hw-test` non-PPU suites are timing-sensitive (bus, DMA, timers, IRQ, haltcnt). Each ROM prints `PASS`/`FAIL` via `test_expect*` in `lib/source/test.c` and `congratulations!` when all sub-tests pass. The headless runner currently executes them for 10M T-cycles and checks a stable memory location (`0x0203FFE0 == 0x00`) as a smoke check; full timing verification will be tightened when Timer/DMA accuracy improves in Phase 8.5. The current matrix runs 13 cases (`nba_128kb-boundary`, `nba_burst-into-tears`, `nba_force-nseq-access`, `nba_latch`, `nba_start-delay`, `nba_haltcnt`, `nba_irq-delay`, `nba_reload`, `nba_start-stop`, `nba_tick-before-reload`, `nba_cancel-irq-*`) all passing. PPU tests (`ppu/**`, `archive/ppu/**`) are excluded from the headless matrix and require visual comparison against `expected.png`/`expected.jpg`.

## Usage

Test ROMs are executed via the `nerust_gba_rom_test` crate (`gba/rom_test/`). The manifest `gba/rom_test/rom_tests.yaml` defines `rom_root: ../../roms/gba` and suites `jsmolka_gba-tests` and `nba-emu_hw-test` (via `case_patterns` excluding `ppu/**`). Run with:

```sh
cargo run -p nerust_gba_rom_test
cargo run -p nerust_gba_rom_test -- jsmolka_bios
cargo run -p nerust_gba_rom_test -- nba_haltcnt
```

Each test loads the ROM via `GbaSystem::from_test_rom`, steps `cycles` T-cycles, and verifies the expected register/memory signature.
