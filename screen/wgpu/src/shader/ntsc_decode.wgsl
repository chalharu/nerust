const NTSC_ROW_OFFSETS: array<array<i32, 6>, 7> = array<array<i32, 6>, 7>(
    array<i32, 6>(0, 19, 31, 7, 26, 38),
    array<i32, 6>(1, 20, 32, 8, 27, 39),
    array<i32, 6>(2, 14, 33, 9, 21, 40),
    array<i32, 6>(3, 15, 34, 10, 22, 41),
    array<i32, 6>(4, 16, 28, 11, 23, 35),
    array<i32, 6>(5, 17, 29, 12, 24, 36),
    array<i32, 6>(6, 18, 30, 13, 25, 37),
);

const NTSC_SOURCE_OFFSETS: array<array<i32, 6>, 7> = array<array<i32, 6>, 7>(
    array<i32, 6>(1, -1, 0, -2, -4, -3),
    array<i32, 6>(1, -1, 0, -2, -4, -3),
    array<i32, 6>(1, 2, 0, -2, -1, -3),
    array<i32, 6>(1, 2, 0, -2, -1, -3),
    array<i32, 6>(1, 2, 3, -2, -1, 0),
    array<i32, 6>(1, 2, 3, -2, -1, 0),
    array<i32, 6>(1, 2, 3, -2, -1, 0),
);

const NTSC_ENTRY_STRIDE: i32 = 42;
const NTSC_CLAMP_MASK: u32 = 0x300c03u;
const NTSC_CLAMP_ADD: u32 = 0x20280a02u;

fn ntsc_entry(color: u32, row: i32) -> u32 {
    return textureLoad(ntsc_texture, vec2<i32>(i32(color), row), 0).r;
}

fn clamp_impl(io: u32) -> u32 {
    let sub = (io >> 9u) & NTSC_CLAMP_MASK;
    let clamp = NTSC_CLAMP_ADD - sub;
    return (io | clamp) & (clamp - sub);
}

fn rgb_out_impl(raw: u32) -> vec3<u32> {
    let rgb = ((raw >> 5u) & 0x00ff0000u) | ((raw >> 3u) & 0x0000ff00u) | ((raw >> 1u) & 0x000000ffu);
    return vec3<u32>((rgb >> 16u) & 0xffu, (rgb >> 8u) & 0xffu, rgb & 0xffu);
}

fn ntsc_rgb_for_output(output: vec2<i32>) -> vec3<u32> {
    let chunk = output.x / 7;
    let sample = output.x - chunk * 7;
    let base = chunk * 3;
    let phase_row = (output.y % 3) * NTSC_ENTRY_STRIDE;
    let source_offsets = NTSC_SOURCE_OFFSETS[u32(sample)];
    let colors = array<u32, 6>(
        palette_index(base + source_offsets[0], output.y),
        palette_index(base + source_offsets[1], output.y),
        palette_index(base + source_offsets[2], output.y),
        palette_index(base + source_offsets[3], output.y),
        palette_index(base + source_offsets[4], output.y),
        palette_index(base + source_offsets[5], output.y),
    );
    let offsets = NTSC_ROW_OFFSETS[u32(sample)];
    let entries = array<u32, 6>(
        ntsc_entry(colors[0], phase_row + offsets[0]),
        ntsc_entry(colors[1], phase_row + offsets[1]),
        ntsc_entry(colors[2], phase_row + offsets[2]),
        ntsc_entry(colors[3], phase_row + offsets[3]),
        ntsc_entry(colors[4], phase_row + offsets[4]),
        ntsc_entry(colors[5], phase_row + offsets[5]),
    );
    let sum = entries[0] + entries[1] + entries[2] + entries[3] + entries[4] + entries[5];
    return rgb_out_impl(clamp_impl(sum));
}
