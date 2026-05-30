# Building

## Prerequisites

- Python 3

No external packages or assembler toolchains are required.

## Build

```bash
python3 generate_rom.py
```

Output:

- `build/Dsp1Smoke.sfc` - 64 KiB LoROM DSP-1 test ROM
- `build/Dsp1aSmoke.sfc` - 64 KiB LoROM DSP-1A-compatible test ROM
- `build/Dsp1bSmoke.sfc` - 64 KiB HiROM DSP-1B test ROM
- `build/Dsp1GeometrySmoke.sfc` - 64 KiB LoROM DSP-1 geometry test ROM
- `build/Dsp1aGeometrySmoke.sfc` - 64 KiB LoROM DSP-1A-compatible geometry test ROM
- `build/Dsp1bGeometrySmoke.sfc` - 64 KiB HiROM DSP-1B geometry test ROM

The generator writes the 65C816 machine code and SNES headers directly so the
artifacts are deterministic and easy to reproduce in CI or locally.
