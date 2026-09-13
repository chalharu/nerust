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

Registered in `gba/rom_test/rom_tests.yaml` (`mgba-suite` suite, one case per suite under test). Cases drive the menu with the input `script` (DOWN taps + A, no buttons held during runs) and branch the debug log into per-subtest checks via `verify.suite_log`, enriched with the SRAM `Got X vs Y` details. The mGBA debug MMIO backing this lives in `GbaMemoryBus` (`0x04FFF600/700/780` + `drain_mgba_debug_logs`), quarantined behind the `mgba-debug-log` cargo feature (test harness only; frontends build without it and see plain open bus).

Status: 13 of 14 suites registered (video emits no log lines and is out of scope for log verification):
- fully passing: shifter, carry, multiply_long (modulo 20 MULLS-C quirk pins), bios_math, misc_edge (modulo 6), io_read (modulo 12 APU masks), sio_read (25/90, rest needs SIO/JOY emulation), dma (1196/1244, rest is the GamePak first-word quirk), timer_irq (modulo 36 start-latency drift), sio_timing (modulo 4 transfer durations).
- `mgba_suite_timing`: whole-case expected failure (1030/1244 fail on systematic bus-timing drift; timing-model overhaul needed).
- `mgba_suite_timers`: quarantined hang (never reaches END; overflow-IRQ storm vs nested-dispatch unwind under diagnosis, fast Timeout budget).

Note: the shipped `suite.gba` (suite-latest, 2026-07-09) is one day newer than the `repo/` snapshot: its SIO suite runs 6 mode groups (M/N8/N32/U/G/J, 90 subtests), not 4.

## License

MPL-2.0 (same as `nerust`).
