# Building

## Prerequisites

- Python 3

No external packages or assembler toolchains are required.

## Build

```bash
python3 generate_rom.py
```

Output:

- `build/Cx4Smoke.sfc` - 64 KiB LoROM CX4 test ROM

The generator writes the 65C816 machine code and SNES header directly so the
artifact is deterministic and easy to reproduce in CI or locally.
