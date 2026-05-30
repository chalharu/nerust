# SNES DSP-1 Family Smoke Test ROMs

Small self-authored SNES DSP-1 family test ROMs for the nerust `rom_test`
harness.

The generated ROMs run on the S-CPU and verify minimal host-visible DSP-1
behavior through the cartridge DSP data/status ports:

- DSP-1 LoROM header detection (`map mode $20`, chipset `$03`)
- DSP-1A-compatible LoROM header detection (`map mode $30`, chipset `$05`)
- DSP-1B HiROM header detection (`map mode $21`, chipset `$05`)
- DSP command `$00` fixed-point multiply
- DSP command `$27` memory-size/ROM-version response
- reset-ready status after command completion

Results are copied into WRAM `$7E:0000-$7E:0004` for manifest assertions. The
ROMs leave display output blank; `rom_test` still captures deterministic final
screen hashes and screenshots.

## License

Public domain. Use freely for emulator testing, development, and validation.
