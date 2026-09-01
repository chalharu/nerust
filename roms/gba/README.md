# GBA Test ROMs

## Upstreams

| Directory | Source | License |
|---|---|---|
| `jsmolka_gba-tests/` | https://github.com/jsmolka/gba-tests | MIT |
| `nba-emu_hw-test/` | https://codeberg.org/nba-emu/hw-test | BSD 3-Clause |
| `armwrestler-gba-fixed/` | https://github.com/destoer/armwrestler-gba-fixed | MIT (assumed) |
| `PeterLemon-GBA/` | https://github.com/PeterLemon/GBA | MIT (assumed) |

Each upstream is imported as a squashed Git subtree.

- `jsmolka_gba-tests`: squashed from commit `a7113b67` (merge `b1693061` / `e221ce14`)
- `nba-emu_hw-test`: squashed from commit `fbc99140e06f083c0a47612467cfbb02470e56dc` (squash `8b81fc11`)
- `armwrestler-gba-fixed`: squashed from commit `802e55a` (master, destoer/armwrestler-gba-fixed)
- `PeterLemon-GBA`: squashed from commit `efdb535` (master, PeterLemon/GBA)

## Build and artifact provenance

- jsmolka `gba-tests` ROMs are assembled with [FASMARM](https://arm.flatassembler.net/). The repository includes prebuilt `.gba` files; rebuild with `fasmarm` if needed.
- `nba-emu/hw-test` ROMs are built with `devkitARM` (`arm-none-eabi-gcc` + `libgba`). Each test directory contains a `Makefile` requiring `DEVKITARM`; prebuilt `.gba` files are included in the subtree and used directly.

## Pass criteria

- `jsmolka_gba-tests` aggregate ROMs (`arm`, `thumb`, `memory`, `save/*`, `bios`) leave the first failed test number in `R12` (`thumb` uses `R7`). `0` means all embedded tests passed. The current matrix runs 8 cases (`jsmolka_arm`, `jsmolka_thumb`, `jsmolka_memory`, `jsmolka_bios`, `jsmolka_save_*`) all passing.
- `nba-emu/hw-test` suites print `PASS`/`FAIL` via `test_expect*` and store `test_count`/`test_pass_count` in IWRAM. The headless matrix verifies those counters for the three Timer ROMs plus `dma/start-delay` and `dma/latch`; all registered sub-tests must pass. `dma/force-nseq-access` and `dma/burst-into-tears` remain unregistered because Game Pak NSEQ restart timing and 128MB-boundary DMA sequencing are not yet accurate. PPU timing ROMs that require HBlank IRQ/DMA remain pending full-frame comparison.
- `armwrestler-gba-fixed` (`armwrestler-gba-fixed.gba`, `armwrestler.gba`) is an interactive menu-driven ARM7TDMI instruction test (Mic 2004, Normmatt 2012, destoer fixed). The headless matrix runs one case `armwrestler_menu` that checks the Mode 3 menu frame (`0x06000000` bitmap, `0x03000008` TESTNUM=10) renders the border at `0,0` as black (`0x0000`) after `2M` T-cycles.
- `PeterLemon-GBA` is a collection of 76 bare-metal demos (FASMARM, krom/Peter Lemon) covering BIOS calls, 3D, sound etc. Each `*.gba` has a prebuilt `*.png` reference (240x160) and is run headless for `1M` T-cycles with automatic `expected.png` diff; demos without reference are checked only for not crashing (`0,0` black).

## Usage

Test ROMs are executed via the `nerust_gba_rom_test` crate (`gba/rom_test/`). The manifest `gba/rom_test/rom_tests.yaml` defines `rom_root: ../../roms/gba` and suites `jsmolka_gba-tests`, `nba-emu_hw-test`, `armwrestler-gba-fixed` and `PeterLemon-GBA`. Run with:

```sh
cargo run -p nerust_gba_rom_test
cargo run -p nerust_gba_rom_test -- jsmolka_bios
cargo run -p nerust_gba_rom_test -- nba_haltcnt
```

Each test loads the ROM via `GbaSystem::from_test_rom`, steps `cycles` T-cycles, and verifies the expected register/memory signature.
