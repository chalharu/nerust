# SNES SA-1 BWRAM Smoke Test ROM

A small self-authored SNES SA-1 test ROM for the nerust `rom_test`
harness.

The program runs on the S-CPU with an SA-1 cartridge header and verifies a
minimal set of SA-1 host-visible behavior:

- SA-1 cartridge header detection (`map mode $23`, chipset `$34`)
- default S-CPU BWRAM write protection
- `$2226` S-CPU BWRAM write-enable behavior
- `$2224` BMAPS page selection for `$00:6000-$00:7FFF`

Results are copied into WRAM `$7E:0000-$7E:0003` for the manifest assertions.
The ROM intentionally leaves display output blank; `rom_test` still captures a
deterministic final screen hash and screenshot.

## License

Public domain. Use freely for emulator testing, development, and validation.
