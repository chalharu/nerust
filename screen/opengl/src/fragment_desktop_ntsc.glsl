uniform usampler2D ntsc_texture;

const int BLACK_INDEX = 15;
const int NTSC_ENTRY_STRIDE = 42;
const uint NTSC_CLAMP_MASK = 0x300c03u;
const uint NTSC_CLAMP_ADD = 0x20280a02u;

int palette_index(int x, int y) {
    if (x < 0 || y < 0 || x >= source_width || y >= source_height) {
        return BLACK_INDEX;
    }
    return int(texelFetch(frame_texture, ivec2(x, y), 0).r);
}

uint ntsc_entry(int color, int row) {
    uvec4 packed = texelFetch(ntsc_texture, ivec2(color, row), 0);
    return (packed.r << 24u) | (packed.g << 16u) | (packed.b << 8u) | packed.a;
}

uint clamp_impl(uint io) {
    uint sub = (io >> 9u) & NTSC_CLAMP_MASK;
    uint clamp = NTSC_CLAMP_ADD - sub;
    return (io | clamp) & (clamp - sub);
}

vec3 rgb_out_impl(uint raw) {
    uint rgb = ((raw >> 5u) & 0x00ff0000u) | ((raw >> 3u) & 0x0000ff00u) | ((raw >> 1u) & 0x000000ffu);
    return vec3(
        float((rgb >> 16u) & 0xffu),
        float((rgb >> 8u) & 0xffu),
        float(rgb & 0xffu)
    ) / 255.0;
}

vec3 decoded_rgb_for_output(ivec2 output_pos) {
    int chunk = output_pos.x / 7;
    int sample = output_pos.x - chunk * 7;
    int base = chunk * 3;
    int phase_row = (output_pos.y % 3) * NTSC_ENTRY_STRIDE;
    int curr0 = palette_index(base + 1, output_pos.y);
    int curr1;
    int curr2;
    int prev0 = palette_index(base - 2, output_pos.y);
    int prev1;
    int prev2;
    if (sample < 2) {
        curr1 = palette_index(base - 1, output_pos.y);
        curr2 = palette_index(base, output_pos.y);
        prev1 = palette_index(base - 4, output_pos.y);
        prev2 = palette_index(base - 3, output_pos.y);
    } else if (sample < 4) {
        curr1 = palette_index(base + 2, output_pos.y);
        curr2 = palette_index(base, output_pos.y);
        prev1 = palette_index(base - 1, output_pos.y);
        prev2 = palette_index(base - 3, output_pos.y);
    } else {
        curr1 = palette_index(base + 2, output_pos.y);
        curr2 = palette_index(base + 3, output_pos.y);
        prev1 = palette_index(base - 1, output_pos.y);
        prev2 = palette_index(base, output_pos.y);
    }
    uint entry0 = ntsc_entry(curr0, phase_row + sample);
    uint entry1 = ntsc_entry(curr1, phase_row + ((sample + 12) % 7 + 14));
    uint entry2 = ntsc_entry(curr2, phase_row + ((sample + 10) % 7 + 28));
    uint entry3 = ntsc_entry(prev0, phase_row + ((sample + 7) % 14));
    uint entry4 = ntsc_entry(prev1, phase_row + ((sample + 5) % 7 + 21));
    uint entry5 = ntsc_entry(prev2, phase_row + ((sample + 3) % 7 + 35));
    return rgb_out_impl(clamp_impl(entry0 + entry1 + entry2 + entry3 + entry4 + entry5));
}
