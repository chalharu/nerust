use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn bit_unpack(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let src = regs.r(0);
    let dst = regs.r(1);
    let info_ptr = regs.r(2);

    if !valid_source(src) || info_ptr & 3 != 0 {
        return;
    }

    let src_len = bus.read16(info_ptr) as u32;
    let src_width = bus.read8(info_ptr + 2) as u32;
    let dst_width = bus.read8(info_ptr + 3) as u32;
    let offset_and_flag = bus.read32(info_ptr + 4);
    let offset = offset_and_flag & 0x7FFFFFFF;
    let zero_flag = (offset_and_flag >> 31) & 1 != 0;

    if src_len == 0
        || !matches!(src_width, 1 | 2 | 4 | 8)
        || !matches!(dst_width, 1 | 2 | 4 | 8 | 16 | 32)
    {
        return;
    }

    let src_mask = (1u32 << src_width) - 1;
    let dst_mask = if dst_width == 32 {
        u32::MAX
    } else {
        (1u32 << dst_width) - 1
    };
    let mut dst_addr = dst;
    let mut dst_bits = 0u32;
    let mut dst_word = 0u32;

    for src_pos in 0..src_len {
        let src_byte = u32::from(bus.read8(src + src_pos));
        for bit in (0..8).step_by(src_width as usize) {
            let value = (src_byte >> bit) & src_mask;
            let adjusted = if value != 0 || zero_flag {
                value.wrapping_add(offset)
            } else {
                value
            } & dst_mask;
            dst_word |= adjusted << dst_bits;
            dst_bits += dst_width;
            if dst_bits == 32 {
                bus.write32(dst_addr, dst_word);
                dst_addr = dst_addr.wrapping_add(4);
                dst_word = 0;
                dst_bits = 0;
            }
        }
    }
    if dst_bits > 0 {
        bus.write32(dst_addr, dst_word);
    }
}

pub fn lz77(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, width: u8) {
    let src = regs.r(0);
    let dst = regs.r(1);

    if !valid_source(src) {
        return;
    }

    let header = bus.read32(src & !3);
    let comp_type = header & 0xF0;
    let decomp_size = header >> 8;
    if comp_type != 0x10 || decomp_size == 0 {
        return;
    }

    let mut src_pos = src + 4;
    let mut remaining = decomp_size;
    let mut output = Vec::with_capacity(decomp_size as usize);

    while remaining > 0 {
        let flag = bus.read8(src_pos);
        src_pos += 1;
        for i in 0..8 {
            if remaining == 0 {
                break;
            }
            if (flag >> (7 - i)) & 1 == 0 {
                let b = bus.read8(src_pos);
                src_pos += 1;
                output.push(b);
                remaining -= 1;
            } else {
                let b0 = bus.read8(src_pos) as u32;
                let b1 = bus.read8(src_pos + 1) as u32;
                src_pos += 2;
                let length = (b0 >> 4) + 3;
                let disp = ((((b0 & 0xF) << 8) | b1) + 1) as usize;
                if disp > output.len() {
                    return;
                }
                for _ in 0..length {
                    if remaining == 0 {
                        break;
                    }
                    let b = output[output.len() - disp];
                    output.push(b);
                    remaining -= 1;
                }
            }
        }
    }
    write_output(bus, dst, &output, width);
}

pub fn huff(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let src = regs.r(0);
    let dst = regs.r(1);

    if !valid_source(src) {
        return;
    }

    let header = bus.read32(src & !3);
    let data_bits = header & 0xF;
    let decomp_size = header >> 8;
    if header & 0xF0 != 0x20 || !matches!(data_bits, 4 | 8) || decomp_size == 0 {
        return;
    }

    let tree_size = bus.read8(src + 4) as u32;
    let tree_bytes = (tree_size + 1) * 2;
    let mut tree_table = vec![0u8; tree_bytes as usize];
    for i in 0..tree_bytes {
        tree_table[i as usize] = bus.read8(src + 5 + i);
    }

    let mut bitstream_pos = (src + 5 + tree_bytes + 3) & !3;
    let mut bit_buffer = bus.read32(bitstream_pos);
    let mut bit_pos = 31;
    let mut out_word = 0u32;
    let mut out_bits = 0u32;
    let target_bits = decomp_size * 8;

    let mut node_addr = 0usize;
    while out_bits < target_bits {
        let bit = (bit_buffer >> bit_pos) & 1;
        if bit_pos == 0 {
            bitstream_pos += 4;
            bit_buffer = bus.read32(bitstream_pos);
            bit_pos = 31;
        } else {
            bit_pos -= 1;
        }

        let node = tree_table[node_addr];
        let offset = (node & 0x3F) as usize;
        let is_end0 = node & 0x80 != 0;
        let is_end1 = node & 0x40 != 0;

        let child = (node_addr & !1) + (offset + 1) * 2 + bit as usize;
        if child >= tree_table.len() {
            return;
        }
        let is_leaf = if bit == 0 { is_end0 } else { is_end1 };
        if is_leaf {
            let symbol = u32::from(tree_table[child]) & if data_bits == 4 { 0xF } else { 0xFF };
            out_word |= symbol << (out_bits % 32);
            out_bits += data_bits;
            if out_bits.is_multiple_of(32) {
                let out_addr = dst + (out_bits / 8) - 4;
                bus.write32(out_addr, out_word);
                out_word = 0;
            }
            node_addr = 0;
        } else {
            node_addr = child;
        }
    }
    if !out_bits.is_multiple_of(32) {
        bus.write32(dst + (out_bits / 32) * 4, out_word);
    }
}

pub fn rl(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, width: u8) {
    let src = regs.r(0);
    let dst = regs.r(1);

    if !valid_source(src) {
        return;
    }

    let header = bus.read32(src & !3);
    let decomp_size = header >> 8;
    if header & 0xF0 != 0x30 || decomp_size == 0 {
        return;
    }

    let mut src_pos = src + 4;
    let mut remaining = decomp_size;
    let mut output = Vec::with_capacity(decomp_size as usize);

    while remaining > 0 {
        let flag = bus.read8(src_pos);
        src_pos += 1;
        let is_compressed = flag & 0x80 != 0;
        let count = if is_compressed {
            ((flag & 0x7F) as u32) + 3
        } else {
            ((flag & 0x7F) as u32) + 1
        };

        if is_compressed {
            let data = bus.read8(src_pos);
            src_pos += 1;
            for _ in 0..count {
                if remaining == 0 {
                    break;
                }
                output.push(data);
                remaining -= 1;
            }
        } else {
            for _ in 0..count {
                if remaining == 0 {
                    break;
                }
                let data = bus.read8(src_pos);
                src_pos += 1;
                output.push(data);
                remaining -= 1;
            }
        }
    }
    write_output(bus, dst, &output, width);
}

fn valid_source(src: u32) -> bool {
    (0x02000000..=0x0FFFFFFF).contains(&src)
}

fn write_output(bus: &mut GbaMemoryBus, dst: u32, output: &[u8], width: u8) {
    if width == 2 {
        for (i, pair) in output.chunks_exact(2).enumerate() {
            bus.write16(
                dst.wrapping_add((i as u32) * 2),
                u16::from_le_bytes([pair[0], pair[1]]),
            );
        }
    } else {
        for (i, &byte) in output.iter().enumerate() {
            bus.write8(dst.wrapping_add(i as u32), byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs_for(src: u32, dst: u32) -> CpuRegisters {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, src);
        regs.set_r(1, dst);
        regs
    }

    #[test]
    fn bit_unpack_supports_32_bit_output() {
        let mut bus = GbaMemoryBus::new();
        let mut regs = regs_for(0x02000000, 0x03000000);
        regs.set_r(2, 0x03000100);
        bus.write8(0x02000000, 0x7F);
        bus.write16(0x03000100, 1);
        bus.write8(0x03000102, 8);
        bus.write8(0x03000103, 32);
        bus.write32(0x03000104, 0);
        bit_unpack(&mut regs, &mut bus);
        assert_eq!(bus.read32(0x03000000), 0x7F);
    }

    #[test]
    fn lz77_wram_and_vram_odd() {
        let mut bus = GbaMemoryBus::new();
        let src = 0x02000000;
        bus.write32(src, 0x00000310);
        bus.write8(src + 4, 0);
        bus.write8(src + 5, b'A');
        bus.write8(src + 6, b'B');
        bus.write8(src + 7, b'C');

        let mut regs = regs_for(src, 0x03000000);
        lz77(&mut regs, &mut bus, 1);
        assert_eq!(bus.read8(0x03000000), b'A');
        assert_eq!(bus.read8(0x03000002), b'C');

        regs.set_r(1, 0x06000000);
        lz77(&mut regs, &mut bus, 2);
        assert_eq!(bus.read16(0x06000000), u16::from_le_bytes(*b"AB"));
        assert_eq!(bus.read8(0x06000002), 0);
    }

    #[test]
    fn huffman_uses_selected_leaf() {
        let mut bus = GbaMemoryBus::new();
        let src = 0x02000000;
        bus.write32(src, 0x00000424); // 4 output bytes, Huffman 4-bit
        bus.write8(src + 4, 1); // 4-byte tree
        bus.write8(src + 5, 0xC0); // both children are leaves
        bus.write8(src + 6, 0);
        bus.write8(src + 7, 0x0A);
        bus.write8(src + 8, 0x0B);
        bus.write32(src + 12, 0x55000000); // left/right alternating
        let mut regs = regs_for(src, 0x03000000);
        huff(&mut regs, &mut bus);
        assert_eq!(bus.read32(0x03000000), 0xBABABABA);
    }

    #[test]
    fn rle_rejects_wrong_header_and_decodes_run() {
        let mut bus = GbaMemoryBus::new();
        let src = 0x02000000;
        let mut regs = regs_for(src, 0x03000000);
        bus.write32(src, 0x00000410);
        rl(&mut regs, &mut bus, 1);
        assert_eq!(bus.read32(0x03000000), 0);

        bus.write32(src, 0x00000430);
        bus.write8(src + 4, 0x81);
        bus.write8(src + 5, 0xAA);
        rl(&mut regs, &mut bus, 1);
        assert_eq!(bus.read32(0x03000000), 0xAAAAAAAA);
    }
}
