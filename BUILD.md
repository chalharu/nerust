# Building

## Prerequisites

- **Python 3** (any version, no packages needed)
- **WSL** or native Linux (the included tool binaries are Linux ELF + Windows PE)

The assembler toolchain is bundled in `tools/`:
- [WLA-DX](https://github.com/vhelin/wla-dx) v9.3 — `wla-65816` (65816 assembler) + `wlalink` (linker)
- [Bass](https://github.com/ARM9/bass) v18 — `bass.exe` (GSU assembler, runs via WSL interop)

## Build

```bash
# WSL or Linux:
make

# Windows PowerShell:
wsl -e bash -lc "cd /mnt/path/to/SNES-HiRomGsuTest && make"
```

Output:
- `build/HiRomGsuTest.sfc` — 4 MB ROM
- `build/HiRomGsuTest.msu` — 4 KB MSU-1 companion data file

## Build Pipeline

```
gen_font.py ────────────────► build/font.bin          (760B, 1bpp 8x8 font)
pixel_test.gsu ───[bass]───► build/pixel_test.bin     (26B, GSU test binary)
gsu_demo.gsu ─────[bass]───► build/gsu_demo.bin       (103B, GSU demo binary)
test_rom.65816 ───[wla]────► build/test_rom.o
test_rom.o ───────[wlalink]─► build/HiRomGsuTest.sfc  (4 MB)
                  [python]──► inject bank signatures
                  [python]──► build/HiRomGsuTest.msu   (4 KB)
```

## Clean

```bash
make clean
```

## Distribution

Place these files together for emulator/hardware testing:

```
HiRomGsuTest.sfc        # ROM (required)
HiRomGsuTest.msu        # MSU-1 data (required for MSU-1 detection)
hirom_gsu_test.bml       # bsnes manifest (optional, bsnes only)
```

## File Structure

```
test_rom.65816           Main 65816 assembly (code, strings, palettes, data)
test_rom.h               WLA-DX memory map (4MB HiROM, 64 banks)
pixel_test.gsu           GSU-2 pixel plot test (Bass syntax)
gsu_demo.gsu             GSU-2 rainbow demo (Bass syntax)
gen_font.py              8x8 1bpp font generator
inject_signatures.py     Post-link: writes bank ID at $FFA0 per bank
makefile                 Build orchestration
linkfile.lnk             WLA-DX linker configuration
hirom_gsu_test.bml       bsnes board manifest
tools/
  bass/bass.exe          Bass v18 assembler (Windows PE, runs via WSL interop)
  bass/architectures/    GSU instruction set definition
  wla-dx/wla-65816       WLA-DX v9.3 assembler (Linux ELF)
  wla-dx/wlalink         WLA-DX v9.3 linker (Linux ELF)
```
