# SNES CX4 Smoke Test ROM

A small self-authored SNES CX4 test ROM for the nerust `rom_test` harness.

The program runs on the S-CPU with a CX4 cartridge header and verifies a
minimal set of host-visible CX4 behavior:

- CX4 cartridge header detection (`map mode $20`, chipset `$F3`, subtype `$10`)
- CX4 24-bit multiply command `$25`
- CX4 identification command `$89`
- CX4 busy-status reads

Results are copied into WRAM `$7E:0000-$7E:0006` for manifest assertions. The
ROM leaves display output blank; `rom_test` still captures a deterministic
final screen hash and screenshot.

## License

Public domain. Use freely for emulator testing, development, and validation.
