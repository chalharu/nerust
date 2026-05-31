const NTSC_ENTRY_STRIDE: i32 = 42;
const NTSC_CLAMP_MASK: u32 = 0x300c03u;
const NTSC_CLAMP_ADD: u32 = 0x20280a02u;

fn ntsc_row_offset(sample: i32, tap: i32) -> i32 {
    if tap == 0 {
        return sample;
    }
    if tap == 1 {
        if sample < 2 {
            return sample + 19;
        }
        return sample + 12;
    }
    if tap == 2 {
        if sample < 4 {
            return sample + 31;
        }
        return sample + 24;
    }
    if tap == 3 {
        return sample + 7;
    }
    if tap == 4 {
        if sample < 2 {
            return sample + 26;
        }
        return sample + 19;
    }
    if sample < 4 {
        return sample + 38;
    }
    return sample + 31;
}

fn ntsc_source_offset(sample: i32, tap: i32) -> i32 {
    if tap == 0 {
        return 1;
    }
    if tap == 1 {
        if sample < 2 {
            return -1;
        }
        return 2;
    }
    if tap == 2 {
        if sample < 4 {
            return 0;
        }
        return 3;
    }
    if tap == 3 {
        return -2;
    }
    if tap == 4 {
        if sample < 2 {
            return -4;
        }
        return -1;
    }
    if sample < 4 {
        return -3;
    }
    return 0;
}

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
    let sum =
        ntsc_entry(
            palette_index(base + ntsc_source_offset(sample, 0), output.y),
            phase_row + ntsc_row_offset(sample, 0),
        )
        + ntsc_entry(
            palette_index(base + ntsc_source_offset(sample, 1), output.y),
            phase_row + ntsc_row_offset(sample, 1),
        )
        + ntsc_entry(
            palette_index(base + ntsc_source_offset(sample, 2), output.y),
            phase_row + ntsc_row_offset(sample, 2),
        )
        + ntsc_entry(
            palette_index(base + ntsc_source_offset(sample, 3), output.y),
            phase_row + ntsc_row_offset(sample, 3),
        )
        + ntsc_entry(
            palette_index(base + ntsc_source_offset(sample, 4), output.y),
            phase_row + ntsc_row_offset(sample, 4),
        )
        + ntsc_entry(
            palette_index(base + ntsc_source_offset(sample, 5), output.y),
            phase_row + ntsc_row_offset(sample, 5),
        );
    return rgb_out_impl(clamp_impl(sum));
}
