# SNES APU DSP Register Smoke Test ROM

A small self-authored SNES APU test ROM for the nerust `rom_test` harness.

The S-CPU uploads a tiny SPC700 program through the IPL protocol. The SPC700
program disables the IPL overlay, writes and reads APU DSP register data through
`$F2/$F3`, exercises the auxiliary APU IO bytes at `$F8/$F9`, and reports the
observed values back through CPU-visible APU ports. The S-CPU copies the final
ports into WRAM `$7E:0000-$7E:0003` for manifest assertions, while the manifest
also checks the APU RAM result bytes.

The ROM leaves display output blank; `rom_test` still captures a deterministic
final screen hash and screenshot.

## License

Public domain. Use freely for emulator testing, development, and validation.
