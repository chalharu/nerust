#!/usr/bin/env python3

from pathlib import Path

ROM_SIZE = 64 * 1024
HEADER_OFFSET = 0x7FC0
RESET_VECTOR_OFFSET = 0x7FFC
PROGRAM_ADDRESS = 0x8000
PROGRAM_OFFSET = PROGRAM_ADDRESS - 0x8000
OUTPUT_PATH = Path(__file__).resolve().parent / "build" / "Sa1BwramSmoke.sfc"


def u16(value):
    return [value & 0xFF, (value >> 8) & 0xFF]


def lda_imm(value):
    return [0xA9, value & 0xFF]


def sta_abs(address):
    return [0x8D, *u16(address)]


def lda_abs(address):
    return [0xAD, *u16(address)]


def sta_long(bank, address):
    return [0x8F, address & 0xFF, (address >> 8) & 0xFF, bank & 0xFF]


def build_program():
    code = []

    # Default BWPA=$0F protects all BWRAM. This write must be ignored.
    code += lda_imm(0xAA)
    code += sta_abs(0x6000)
    code += lda_abs(0x6000)
    code += sta_long(0x7E, 0x0000)

    # Enable S-CPU BWRAM writes and verify page 0 read-back.
    code += lda_imm(0x80)
    code += sta_abs(0x2226)
    code += lda_imm(0x5A)
    code += sta_abs(0x6000)
    code += lda_abs(0x6000)
    code += sta_long(0x7E, 0x0001)

    # Select BMAPS page 1, write/read it, then switch back to page 0.
    code += lda_imm(0x01)
    code += sta_abs(0x2224)
    code += lda_imm(0xA5)
    code += sta_abs(0x6000)
    code += lda_abs(0x6000)
    code += sta_long(0x7E, 0x0002)

    code += lda_imm(0x00)
    code += sta_abs(0x2224)
    code += lda_abs(0x6000)
    code += sta_long(0x7E, 0x0003)

    # Stay alive for benchmark frame loops.
    code += [0x80, 0xFE]
    return bytes(code)


def write_header(rom):
    title = b"NERUST SA1 BWRAM     "
    rom[HEADER_OFFSET : HEADER_OFFSET + len(title)] = title
    rom[HEADER_OFFSET + 0x15] = 0x23  # SA-1 map mode
    rom[HEADER_OFFSET + 0x16] = 0x34  # SA-1 + RAM + battery family
    rom[HEADER_OFFSET + 0x17] = 0x06  # 64 KiB ROM
    rom[HEADER_OFFSET + 0x18] = 0x05  # 32 KiB BWRAM
    rom[HEADER_OFFSET + 0x19] = 0x01  # NTSC
    rom[HEADER_OFFSET + 0x1A] = 0x33  # maker code
    rom[HEADER_OFFSET + 0x1B] = 0x00  # version

    for vector_offset in (0x7FEA, 0x7FEC, 0x7FEE, 0x7FFA, RESET_VECTOR_OFFSET, 0x7FFE):
        rom[vector_offset : vector_offset + 2] = bytes(u16(PROGRAM_ADDRESS))

    rom[HEADER_OFFSET + 0x1C : HEADER_OFFSET + 0x20] = b"\x00\x00\x00\x00"
    checksum = sum(rom) & 0xFFFF
    complement = checksum ^ 0xFFFF
    rom[HEADER_OFFSET + 0x1C : HEADER_OFFSET + 0x1E] = bytes(u16(complement))
    rom[HEADER_OFFSET + 0x1E : HEADER_OFFSET + 0x20] = bytes(u16(checksum))


def main():
    rom = bytearray([0xFF] * ROM_SIZE)
    program = build_program()
    rom[PROGRAM_OFFSET : PROGRAM_OFFSET + len(program)] = program
    write_header(rom)

    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_bytes(rom)
    print(f"wrote {OUTPUT_PATH} ({len(rom)} bytes)")


if __name__ == "__main__":
    main()
