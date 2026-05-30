#!/usr/bin/env python3

from pathlib import Path

ROM_SIZE = 64 * 1024
LOROM_HEADER_OFFSET = 0x7FC0
LOROM_RESET_VECTOR_OFFSET = 0x7FFC
HIROM_HEADER_OFFSET = 0xFFC0
HIROM_RESET_VECTOR_OFFSET = 0xFFFC
PROGRAM_ADDRESS = 0x8000
LOROM_PROGRAM_OFFSET = PROGRAM_ADDRESS - 0x8000
HIROM_PROGRAM_OFFSET = PROGRAM_ADDRESS
OUTPUT_DIR = Path(__file__).resolve().parent / "build"


def u16(value):
    return [value & 0xFF, (value >> 8) & 0xFF]


def lda_imm(value):
    return [0xA9, value & 0xFF]


def lda_long(bank, address):
    return [0xAF, address & 0xFF, (address >> 8) & 0xFF, bank & 0xFF]


def sta_long(bank, address):
    return [0x8F, address & 0xFF, (address >> 8) & 0xFF, bank & 0xFF]


def write_imm_long(value, bank, address):
    return [*lda_imm(value), *sta_long(bank, address)]


def copy_long_to_wram(source_bank, source_address, destination):
    return [*lda_long(source_bank, source_address), *sta_long(0x7E, destination)]


def copy_dsp_word_to_wram(code, data_bank, data_address, destination):
    code += copy_long_to_wram(data_bank, data_address, destination)
    code += copy_long_to_wram(data_bank, data_address, destination + 1)


def write_dsp_word(code, data_bank, data_address, value):
    code += write_imm_long(value & 0xFF, data_bank, data_address)
    code += write_imm_long((value >> 8) & 0xFF, data_bank, data_address)


def build_program(data_bank, data_address, status_bank, status_address):
    code = []

    # Command $00: signed fixed-point multiply. $4000 * $4000 => $2000.
    code += write_imm_long(0x00, data_bank, data_address)
    write_dsp_word(code, data_bank, data_address, 0x4000)
    write_dsp_word(code, data_bank, data_address, 0x4000)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0000)

    # Command $27: memory size / ROM version. DSP-1/1A return $0100, DSP-1B returns $0101.
    code += write_imm_long(0x27, data_bank, data_address)
    write_dsp_word(code, data_bank, data_address, 0x0000)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0002)
    code += copy_long_to_wram(status_bank, status_address, 0x0004)

    # Stay alive for benchmark frame loops.
    code += [0x80, 0xFE]
    return bytes(code)


def build_geometry_program(data_bank, data_address, status_bank, status_address):
    code = []

    # Command $04: sin/cos. angle 0, radius $4000 => sin 0, cos $4000.
    code += write_imm_long(0x04, data_bank, data_address)
    write_dsp_word(code, data_bank, data_address, 0x0000)
    write_dsp_word(code, data_bank, data_address, 0x4000)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0010)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0012)

    # Command $0C: 2D rotation. angle 0 leaves X/Y unchanged.
    code += write_imm_long(0x0C, data_bank, data_address)
    write_dsp_word(code, data_bank, data_address, 0x0000)
    write_dsp_word(code, data_bank, data_address, 0x0123)
    write_dsp_word(code, data_bank, data_address, 0xFEDC)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0014)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0016)

    # Command $1C: 3D rotation. zero Euler angles leave X/Y/Z unchanged.
    code += write_imm_long(0x1C, data_bank, data_address)
    for value in (0x0000, 0x0000, 0x0000, 0x0003, 0xFFFC, 0x000C):
        write_dsp_word(code, data_bank, data_address, value)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0018)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x001A)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x001C)

    # Command $28: vector length. sqrt(3^2 + 4^2 + 12^2) => 13.
    code += write_imm_long(0x28, data_bank, data_address)
    for value in (0x0003, 0x0004, 0x000C):
        write_dsp_word(code, data_bank, data_address, value)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x001E)

    # Command $08: radius/squared-length. 3^2 + 4^2 + 12^2 => $000000A9.
    code += write_imm_long(0x08, data_bank, data_address)
    for value in (0x0003, 0x0004, 0x000C):
        write_dsp_word(code, data_bank, data_address, value)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0020)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0022)

    # Command $10: inverse. coefficient $4000, exponent 0 => $7FFF, $0001.
    code += write_imm_long(0x10, data_bank, data_address)
    write_dsp_word(code, data_bank, data_address, 0x4000)
    write_dsp_word(code, data_bank, data_address, 0x0000)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0024)
    copy_dsp_word_to_wram(code, data_bank, data_address, 0x0026)
    code += copy_long_to_wram(status_bank, status_address, 0x0028)

    # Stay alive for benchmark frame loops.
    code += [0x80, 0xFE]
    return bytes(code)


def write_header(rom, *, header_offset, reset_vector_offset, title, map_mode, chipset, ram_size=0):
    encoded_title = title.encode("ascii")[:21].ljust(21, b" ")

    rom[header_offset : header_offset + len(encoded_title)] = encoded_title
    rom[header_offset + 0x15] = map_mode
    rom[header_offset + 0x16] = chipset
    rom[header_offset + 0x17] = 0x06  # 64 KiB ROM
    rom[header_offset + 0x18] = ram_size
    rom[header_offset + 0x19] = 0x01  # NTSC
    rom[header_offset + 0x1A] = 0x33  # maker code
    rom[header_offset + 0x1B] = 0x00  # version

    for vector_offset in (
        reset_vector_offset - 0x12,
        reset_vector_offset - 0x10,
        reset_vector_offset - 0x0E,
        reset_vector_offset - 0x02,
        reset_vector_offset,
        reset_vector_offset + 0x02,
    ):
        rom[vector_offset : vector_offset + 2] = bytes(u16(PROGRAM_ADDRESS))

    rom[header_offset + 0x1C : header_offset + 0x20] = b"\x00\x00\x00\x00"
    checksum = sum(rom) & 0xFFFF
    complement = checksum ^ 0xFFFF
    rom[header_offset + 0x1C : header_offset + 0x1E] = bytes(u16(complement))
    rom[header_offset + 0x1E : header_offset + 0x20] = bytes(u16(checksum))


def write_rom(filename, program, header, program_offset):
    rom = bytearray([0xFF] * ROM_SIZE)
    rom[program_offset : program_offset + len(program)] = program
    write_header(rom, **header)

    output_path = OUTPUT_DIR / filename
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(rom)
    print(f"wrote {output_path} ({len(rom)} bytes)")


def main():
    write_rom(
        "Dsp1Smoke.sfc",
        build_program(0x20, 0x8000, 0x20, 0xC000),
        {
            "header_offset": LOROM_HEADER_OFFSET,
            "reset_vector_offset": LOROM_RESET_VECTOR_OFFSET,
            "title": "NERUST DSP1 SMOKE   ",
            "map_mode": 0x20,
            "chipset": 0x03,
        },
        LOROM_PROGRAM_OFFSET,
    )
    write_rom(
        "Dsp1aSmoke.sfc",
        build_program(0x20, 0x8000, 0x20, 0xC000),
        {
            "header_offset": LOROM_HEADER_OFFSET,
            "reset_vector_offset": LOROM_RESET_VECTOR_OFFSET,
            "title": "NERUST DSP1A SMOKE  ",
            "map_mode": 0x30,
            "chipset": 0x05,
        },
        LOROM_PROGRAM_OFFSET,
    )
    write_rom(
        "Dsp1bSmoke.sfc",
        build_program(0x00, 0x6000, 0x00, 0x7000),
        {
            "header_offset": HIROM_HEADER_OFFSET,
            "reset_vector_offset": HIROM_RESET_VECTOR_OFFSET,
            "title": "NERUST DSP1B SMOKE  ",
            "map_mode": 0x21,
            "chipset": 0x05,
            "ram_size": 0x02,
        },
        HIROM_PROGRAM_OFFSET,
    )
    write_rom(
        "Dsp1GeometrySmoke.sfc",
        build_geometry_program(0x20, 0x8000, 0x20, 0xC000),
        {
            "header_offset": LOROM_HEADER_OFFSET,
            "reset_vector_offset": LOROM_RESET_VECTOR_OFFSET,
            "title": "NERUST DSP1 GEOM    ",
            "map_mode": 0x20,
            "chipset": 0x03,
        },
        LOROM_PROGRAM_OFFSET,
    )
    write_rom(
        "Dsp1aGeometrySmoke.sfc",
        build_geometry_program(0x20, 0x8000, 0x20, 0xC000),
        {
            "header_offset": LOROM_HEADER_OFFSET,
            "reset_vector_offset": LOROM_RESET_VECTOR_OFFSET,
            "title": "NERUST DSP1A GEOM   ",
            "map_mode": 0x30,
            "chipset": 0x05,
        },
        LOROM_PROGRAM_OFFSET,
    )
    write_rom(
        "Dsp1bGeometrySmoke.sfc",
        build_geometry_program(0x00, 0x6000, 0x00, 0x7000),
        {
            "header_offset": HIROM_HEADER_OFFSET,
            "reset_vector_offset": HIROM_RESET_VECTOR_OFFSET,
            "title": "NERUST DSP1B GEOM   ",
            "map_mode": 0x21,
            "chipset": 0x05,
            "ram_size": 0x02,
        },
        HIROM_PROGRAM_OFFSET,
    )


if __name__ == "__main__":
    main()
