use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn bit_unpack(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let Some(spec) = BitUnpackSpec::read(regs, bus) else {
        return;
    };
    unpack_bits(bus, spec);
}

struct BitUnpackSpec {
    source: u32,
    destination: u32,
    source_len: u32,
    source_width: u32,
    destination_width: u32,
    offset: u32,
    offset_zero: bool,
}

impl BitUnpackSpec {
    fn read(regs: &CpuRegisters, bus: &mut GbaMemoryBus) -> Option<Self> {
        let source = regs.r(0);
        let info = regs.r(2);
        if !valid_source(source) || info & 3 != 0 {
            return None;
        }
        let source_len = u32::from(bus.read16(info));
        let source_width = u32::from(bus.read8(info + 2));
        let destination_width = u32::from(bus.read8(info + 3));
        if source_len == 0
            || !matches!(source_width, 1 | 2 | 4 | 8)
            || !matches!(destination_width, 1 | 2 | 4 | 8 | 16 | 32)
        {
            return None;
        }
        let offset = bus.read32(info + 4);
        Some(Self {
            source,
            destination: regs.r(1),
            source_len,
            source_width,
            destination_width,
            offset: offset & 0x7FFF_FFFF,
            offset_zero: offset >> 31 != 0,
        })
    }
}

fn unpack_bits(bus: &mut GbaMemoryBus, spec: BitUnpackSpec) {
    let source_mask = (1u32 << spec.source_width) - 1;
    let destination_mask = width_mask(spec.destination_width);
    let mut destination = spec.destination;
    let mut dst_bits = 0u32;
    let mut dst_word = 0u32;
    for position in 0..spec.source_len {
        let source_byte = u32::from(bus.read8(spec.source + position));
        for bit in (0..8).step_by(spec.source_width as usize) {
            let value = (source_byte >> bit) & source_mask;
            let adjusted = apply_offset(value, spec.offset, spec.offset_zero) & destination_mask;
            dst_word |= adjusted << dst_bits;
            dst_bits += spec.destination_width;
            if dst_bits == 32 {
                bus.write32(destination, dst_word);
                destination = destination.wrapping_add(4);
                dst_word = 0;
                dst_bits = 0;
            }
        }
    }
    if dst_bits > 0 {
        bus.write32(destination, dst_word);
    }
}

fn width_mask(width: u32) -> u32 {
    if width == 32 {
        u32::MAX
    } else {
        (1 << width) - 1
    }
}

fn apply_offset(value: u32, offset: u32, include_zero: bool) -> u32 {
    if value != 0 || include_zero {
        value.wrapping_add(offset)
    } else {
        value
    }
}

pub fn lz77(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, width: u8) {
    let src = regs.r(0);
    let Some(size) = decompressed_size(bus, src, 0x10) else {
        return;
    };
    let Some(output) = decode_lz77(bus, src + 4, size) else {
        return;
    };
    write_output(bus, regs.r(1), &output, width);
}

fn decode_lz77(bus: &mut GbaMemoryBus, mut source: u32, size: u32) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(size as usize);
    while output.len() < size as usize {
        let flag = bus.read8(source);
        source += 1;
        for i in 0..8 {
            if output.len() == size as usize {
                break;
            }
            if (flag >> (7 - i)) & 1 == 0 {
                output.push(bus.read8(source));
                source += 1;
            } else {
                source = append_lz_reference(bus, source, size as usize, &mut output)?;
            }
        }
    }
    Some(output)
}

fn append_lz_reference(
    bus: &mut GbaMemoryBus,
    source: u32,
    target_len: usize,
    output: &mut Vec<u8>,
) -> Option<u32> {
    let first = u32::from(bus.read8(source));
    let second = u32::from(bus.read8(source + 1));
    let length = ((first >> 4) + 3) as usize;
    let distance = ((((first & 0xF) << 8) | second) + 1) as usize;
    if distance > output.len() {
        return None;
    }
    for _ in 0..length.min(target_len - output.len()) {
        output.push(output[output.len() - distance]);
    }
    Some(source + 2)
}

pub fn huff(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let src = regs.r(0);
    let header = bus.read32(src & !3);
    let data_bits = header & 0xF;
    let Some(size) = valid_huffman_size(src, header, data_bits) else {
        return;
    };
    let tree = read_huffman_tree(bus, src);
    if tree.is_empty() {
        return;
    }
    decode_huffman(bus, src, regs.r(1), data_bits, size, &tree);
}

fn valid_huffman_size(source: u32, header: u32, bits: u32) -> Option<u32> {
    let size = header >> 8;
    (valid_source(source) && header & 0xF0 == 0x20 && matches!(bits, 4 | 8) && size > 0)
        .then_some(size)
}

fn read_huffman_tree(bus: &mut GbaMemoryBus, source: u32) -> Vec<u8> {
    let tree_size = u32::from(bus.read8(source + 4));
    let tree_bytes = (tree_size + 1) * 2;
    let mut tree_table = vec![0u8; tree_bytes as usize];
    for i in 0..tree_bytes {
        tree_table[i as usize] = bus.read8(source + 5 + i);
    }
    tree_table
}

fn decode_huffman(
    bus: &mut GbaMemoryBus,
    source: u32,
    destination: u32,
    data_bits: u32,
    size: u32,
    tree: &[u8],
) {
    let bitstream = (source + 5 + tree.len() as u32 + 3) & !3;
    let mut reader = BitReader::new(bus, bitstream);
    let mut out_word = 0u32;
    let mut out_bits = 0u32;
    let mut node_addr = 0usize;
    while out_bits < size * 8 {
        let bit = reader.next(bus);
        let node = tree[node_addr];
        let offset = (node & 0x3F) as usize;
        let child = (node_addr & !1) + (offset + 1) * 2 + bit as usize;
        if child >= tree.len() {
            return;
        }
        let is_leaf = node & if bit == 0 { 0x80 } else { 0x40 } != 0;
        if is_leaf {
            let symbol = u32::from(tree[child]) & width_mask(data_bits);
            out_word |= symbol << (out_bits % 32);
            out_bits += data_bits;
            if out_bits.is_multiple_of(32) {
                let out_addr = destination + (out_bits / 8) - 4;
                bus.write32(out_addr, out_word);
                out_word = 0;
            }
            node_addr = 0;
        } else {
            node_addr = child;
        }
    }
    if !out_bits.is_multiple_of(32) {
        bus.write32(destination + (out_bits / 32) * 4, out_word);
    }
}

struct BitReader {
    address: u32,
    value: u32,
    position: u32,
}

impl BitReader {
    fn new(bus: &mut GbaMemoryBus, address: u32) -> Self {
        Self {
            address,
            value: bus.read32(address),
            position: 31,
        }
    }

    fn next(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        let bit = (self.value >> self.position) & 1;
        if self.position == 0 {
            self.address += 4;
            self.value = bus.read32(self.address);
            self.position = 31;
        } else {
            self.position -= 1;
        }
        bit
    }
}

pub fn rl(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, width: u8) {
    let src = regs.r(0);
    let Some(size) = decompressed_size(bus, src, 0x30) else {
        return;
    };
    let output = decode_rl(bus, src + 4, size);
    write_output(bus, regs.r(1), &output, width);
}

fn decode_rl(bus: &mut GbaMemoryBus, mut source: u32, size: u32) -> Vec<u8> {
    let mut output = Vec::with_capacity(size as usize);
    while output.len() < size as usize {
        let flag = bus.read8(source);
        source += 1;
        let is_compressed = flag & 0x80 != 0;
        let count = usize::from(flag & 0x7F) + if is_compressed { 3 } else { 1 };
        if is_compressed {
            let value = bus.read8(source);
            source += 1;
            output.extend(std::iter::repeat_n(
                value,
                count.min(size as usize - output.len()),
            ));
        } else {
            for _ in 0..count.min(size as usize - output.len()) {
                output.push(bus.read8(source));
                source += 1;
            }
        }
    }
    output
}

fn decompressed_size(bus: &mut GbaMemoryBus, source: u32, kind: u32) -> Option<u32> {
    if !valid_source(source) {
        return None;
    }
    let header = bus.read32(source & !3);
    let size = header >> 8;
    (header & 0xF0 == kind && size > 0).then_some(size)
}

fn valid_source(src: u32) -> bool {
    (0x02000000..=0x0FFFFFFF).contains(&src)
}

fn write_output(bus: &mut GbaMemoryBus, dst: u32, output: &[u8], width: u8) {
    if width == 2 {
        // BIOS VRAM variants write halfwords; a trailing odd byte is not flushed.
        for (i, pair) in output.as_chunks::<2>().0.iter().enumerate() {
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
