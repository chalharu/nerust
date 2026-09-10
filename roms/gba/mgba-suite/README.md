# mgba-suite

Upstream: https://github.com/mgba-emu/suite (MPL-2.0) — squashed as `repo/` via `git subtree` from `e694203` (master).

This is the upstream **mGBA test suite** for GBA (14 suites: memory, timing, timers, DMA, video, etc.). It is an interactive menu-driven ROM (`suite.gba`) that reports per-subtest `PASS:`/`FAIL:` via `mgba_printf` (`0x4FFF600` string + `0x4FFF700` flags, enabled by the `0x4FFF780` handshake) and `Got X vs Y` details via `savprintf` to SRAM (`0x0E000000`).

`suite.gba` here is the upstream prebuilt `suite-latest.zip` (replaced 2026-09-10), not a local devkitARM build.

## Layout (like `roms/gbc/*/repo`)

```
mgba-suite/
  repo/          # squashed subtree of mgba-emu/suite (master, e694203)
    src/         # 14 suites (memory.c, timing.c, dma.c, video.c, etc.)
    include/     # suite.h, mgba.h, etc.
    gfx/         # font.grit etc.
    Makefile     # requires DEVKITARM + libgba
  suite.gba      # upstream prebuilt under test (committed)
  README.md      # this file
```

## Headless driving

Registered in `gba/rom_test/rom_tests.yaml` (`mgba-suite` suite, one case per suite under test). Cases drive the menu with the input `script` (DOWN taps + A, no buttons held during runs) and branch the debug log into per-subtest checks via `verify.suite_log`, enriched with the SRAM `Got X vs Y` details. The mGBA debug MMIO backing this lives in `GbaMemoryBus` (`0x04FFF600/700/780` + `drain_mgba_debug_logs`).

Status: `mgba_suite_memory` registered (1502/1552 subtests pass; 50 reference-side `expected_checks`). Remaining suites (timing, timers, DMA, ...) are future cases following the same pattern; the video suite emits no log lines and is out of scope for log verification.

## License

MPL-2.0 (same as `nerust`).
