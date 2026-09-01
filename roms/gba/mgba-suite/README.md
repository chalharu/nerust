# mgba-suite

Upstream: https://github.com/mgba-emu/suite (MPL-2.0) — squashed as `repo/` via `git subtree` from `e694203` (master).

This is the upstream **mGBA test suite** for GBA (14 suites: memory, timing, timers, DMA, video, etc.). It is an interactive menu-driven ROM (`suite.gba`) that reports `PASS/FAIL` via `mgba_printf` (`0x4FF600`) and `savprintf` to SRAM (`0x0E000000`), with `IWRAM_DATA activeTestInfo` at `0x03000000`.

## Layout (like `roms/gbc/*/repo`)

```
mgba-suite/
  repo/          # squashed subtree of mgba-emu/suite (master, e694203)
    src/         # 14 suites (memory.c, timing.c, dma.c, video.c, etc.)
    include/     # suite.h, mgba.h, etc.
    gfx/         # font.grit etc.
    Makefile     # requires DEVKITARM + libgba
  README.md      # this file
```

## Build

Like `roms/gbc/*/repo/mgblib` submodules, this suite requires `devkitARM` to build:

```sh
make -C roms/gba/mgba-suite/repo
# produces roms/gba/mgba-suite/repo/suite.gba (ignored, not committed)
# copy to roms/gba/mgba-suite/suite.gba if you want to register it
```

CI will skip the suite if `suite.gba` is missing. Once built, register in `gba/rom_test/rom_tests.yaml`:

```yaml
- name: mgba-suite
  cases:
  - id: mgba_memory
    rom: mgba-suite/suite.gba
    cycles: 3000000
    setup:
      - {address: "0x03000000", value: "0x6F6E49", width: 4} # 'Info' magic for activeTestInfo
```

Headless driving is like `armwrestler-gba-fixed` (`setup` writes to `0x03000008` TESTNUM) — see `armwrestler` handling in `gba/rom_test/src/runner.rs`.

## License

MPL-2.0 (same as `nerust`).
