#!/usr/bin/env python3

from pathlib import Path

ROM_SIZE = 64 * 1024
HEADER_OFFSET = 0x7FC0
RESET_VECTOR_OFFSET = 0x7FFC
PROGRAM_ADDRESS = 0x8000
PROGRAM_OFFSET = PROGRAM_ADDRESS - 0x8000
SPC_ENTRY = 0x0300
OUTPUT_PATH = Path(__file__).resolve().parent / "build" / "ApuDspRegisterSmoke.sfc"


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


def cmp_imm(value):
    return [0xC9, value & 0xFF]


def wait_abs_eq(address, value):
    return [*lda_abs(address), *cmp_imm(value), 0xD0, 0xF9]


def store_imm_abs(value, address):
    return [*lda_imm(value), *sta_abs(address)]


def copy_abs_to_wram(source, destination):
    return [*lda_abs(source), *sta_long(0x7E, destination)]


def build_spc_program():
    return bytes(
        [
            0x8F,
            0x00,
            0xF1,  # MOV $F1,#$00: disable IPL ROM overlay
            0x8F,
            0x0C,
            0xF2,  # MOV $F2,#$0C: select MVOLL
            0x8F,
            0x7F,
            0xF3,  # MOV $F3,#$7F
            0xE4,
            0xF3,  # MOV A,$F3
            0xC4,
            0x20,  # MOV $20,A
            0xC4,
            0xF5,  # MOV $F5,A
            0x8F,
            0x2C,
            0xF2,  # MOV $F2,#$2C: select EVOLL
            0x8F,
            0x40,
            0xF3,  # MOV $F3,#$40
            0xE4,
            0xF3,  # MOV A,$F3
            0xC4,
            0x21,  # MOV $21,A
            0xC4,
            0xF6,  # MOV $F6,A
            0xE4,
            0xF2,  # MOV A,$F2
            0xC4,
            0x22,  # MOV $22,A
            0xC4,
            0xF7,  # MOV $F7,A
            0x8F,
            0x12,
            0xF8,  # MOV $F8,#$12
            0x8F,
            0x34,
            0xF9,  # MOV $F9,#$34
            0xE4,
            0xF8,  # MOV A,$F8
            0xC4,
            0x23,  # MOV $23,A
            0xE4,
            0xF9,  # MOV A,$F9
            0xC4,
            0x24,  # MOV $24,A
            0x8F,
            0xA5,
            0xF4,  # MOV $F4,#$A5: success marker
            0xFF,  # STOP
        ]
    )


def build_program():
    spc_program = build_spc_program()
    if len(spc_program) > 0xFD:
        raise ValueError("SPC program is too large for the one-page IPL uploader")

    code = []
    code += wait_abs_eq(0x2140, 0xAA)
    code += wait_abs_eq(0x2141, 0xBB)
    code += store_imm_abs(SPC_ENTRY & 0xFF, 0x2142)
    code += store_imm_abs(SPC_ENTRY >> 8, 0x2143)
    code += store_imm_abs(0x01, 0x2141)
    code += store_imm_abs(0xCC, 0x2140)
    code += wait_abs_eq(0x2140, 0xCC)

    for index, value in enumerate(spc_program):
        code += store_imm_abs(value, 0x2141)
        code += store_imm_abs(index, 0x2140)
        code += wait_abs_eq(0x2140, index)

    kick = (len(spc_program) + 2) | 1
    code += store_imm_abs(SPC_ENTRY & 0xFF, 0x2142)
    code += store_imm_abs(SPC_ENTRY >> 8, 0x2143)
    code += store_imm_abs(0x00, 0x2141)
    code += store_imm_abs(kick, 0x2140)
    code += wait_abs_eq(0x2140, kick)
    code += wait_abs_eq(0x2140, 0xA5)

    for port in range(4):
        code += copy_abs_to_wram(0x2140 + port, port)

    code += [0x80, 0xFE]
    return bytes(code)


def write_header(rom):
    title = b"NERUST APU DSP SMOKE "
    rom[HEADER_OFFSET : HEADER_OFFSET + len(title)] = title
    rom[HEADER_OFFSET + 0x15] = 0x20  # LoROM map mode
    rom[HEADER_OFFSET + 0x16] = 0x00  # ROM only
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
