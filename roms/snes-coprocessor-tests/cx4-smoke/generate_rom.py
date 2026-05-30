#!/usr/bin/env python3

from pathlib import Path

ROM_SIZE = 64 * 1024
HEADER_OFFSET = 0x7FC0
RESET_VECTOR_OFFSET = 0x7FFC
PROGRAM_ADDRESS = 0x8000
PROGRAM_OFFSET = PROGRAM_ADDRESS - 0x8000
OUTPUT_PATH = Path(__file__).resolve().parent / "build" / "Cx4Smoke.sfc"


def u16(value):
    return [value & 0xFF, (value >> 8) & 0xFF]


def lda_imm(value):
    return [0xA9, value & 0xFF]


def lda_abs(address):
    return [0xAD, *u16(address)]


def sta_abs(address):
    return [0x8D, *u16(address)]


def sta_long(bank, address):
    return [0x8F, address & 0xFF, (address >> 8) & 0xFF, bank & 0xFF]


def store_imm_abs(value, address):
    return [*lda_imm(value), *sta_abs(address)]


def copy_abs_to_wram(source, destination):
    return [*lda_abs(source), *sta_long(0x7E, destination)]


def build_program():
    code = []

    # CX4 command $25 multiplies two little-endian 24-bit inputs at $7F80.
    for value, address in (
        (0x23, 0x7F80),
        (0x01, 0x7F81),
        (0x00, 0x7F82),
        (0x04, 0x7F83),
        (0x00, 0x7F84),
        (0x00, 0x7F85),
    ):
        code += store_imm_abs(value, address)
    code += store_imm_abs(0x25, 0x7F4F)
    code += copy_abs_to_wram(0x7F80, 0x0000)
    code += copy_abs_to_wram(0x7F81, 0x0001)
    code += copy_abs_to_wram(0x7F82, 0x0002)

    # CX4 command $89 returns the CX4 identifier bytes 36 43 05.
    code += store_imm_abs(0x89, 0x7F4F)
    code += copy_abs_to_wram(0x7F80, 0x0003)
    code += copy_abs_to_wram(0x7F81, 0x0004)
    code += copy_abs_to_wram(0x7F82, 0x0005)
    code += copy_abs_to_wram(0x7F5E, 0x0006)

    # Stay alive for benchmark frame loops.
    code += [0x80, 0xFE]
    return bytes(code)


def write_header(rom):
    title = b"NERUST CX4 SMOKE     "
    rom[HEADER_OFFSET - 1] = 0x10  # CX4 expansion subtype
    rom[HEADER_OFFSET : HEADER_OFFSET + len(title)] = title
    rom[HEADER_OFFSET + 0x15] = 0x20  # LoROM map mode
    rom[HEADER_OFFSET + 0x16] = 0xF3  # coprocessor family with enhancement subtype
    rom[HEADER_OFFSET + 0x17] = 0x06  # 64 KiB ROM
    rom[HEADER_OFFSET + 0x18] = 0x00  # no cartridge RAM
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
