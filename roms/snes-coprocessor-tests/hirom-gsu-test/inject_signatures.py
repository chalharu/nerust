#!/usr/bin/env python3
"""Inject per-bank signatures into the ROM for bank mapping verification.

Writes the bank number (0-63) at offset $FFA0 in each 64KB bank.
The test ROM reads these back at runtime to verify HiROM bank mapping.
"""
import sys

rom = bytearray(open(sys.argv[1], 'rb').read())
num_banks = len(rom) // 0x10000
for bank in range(num_banks):
    rom[bank * 0x10000 + 0xFFA0] = bank
open(sys.argv[1], 'wb').write(rom)
print(f"Injected {num_banks} bank signatures into {sys.argv[1]}")
