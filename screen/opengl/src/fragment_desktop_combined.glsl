uniform sampler2D frame_texture;
uniform sampler2D palette_texture;
uniform usampler2D ntsc_texture;
uniform vec2 source_size;
uniform bool ntsc_enabled;
in vec2 vuv;
out vec4 frag_color;

uint palette_index(ivec2 pos) {
    if (pos.x < 0 || pos.y < 0 || pos.x >= int(source_size.x) || pos.y >= int(source_size.y)) {
        return 15u;
    }
    return uint(round(texelFetch(frame_texture, pos, 0).r * 255.0));
}

vec3 palette_color(uint index) {
    return texelFetch(palette_texture, ivec2(int(index), 0), 0).rgb;
}

// ---- NTSC decode (translated from wgpu ntsc_decode.wgsl) ----

#define NTSC_ENTRY_STRIDE 42

int ntsc_row_offset(int sample, int tap) {
    if (tap == 0) return sample;
    if (tap == 1) return (sample < 2) ? sample + 19 : sample + 12;
    if (tap == 2) return (sample < 4) ? sample + 31 : sample + 24;
    if (tap == 3) return sample + 7;
    if (tap == 4) return (sample < 2) ? sample + 26 : sample + 19;
    return (sample < 4) ? sample + 38 : sample + 31;
}

int ntsc_source_offset(int sample, int tap) {
    if (tap == 0) return 1;
    if (tap == 1) return (sample < 2) ? -1 : 2;
    if (tap == 2) return (sample < 4) ? 0 : 3;
    if (tap == 3) return -2;
    if (tap == 4) return (sample < 2) ? -4 : -1;
    return (sample < 4) ? -3 : 0;
}

uint ntsc_entry(uint color_index, int row) {
    return texelFetch(ntsc_texture, ivec2(int(color_index), row), 0).r;
}

uint clamp_impl(uint io) {
    uint sub = (io >> 9u) & 0x300c03u;
    uint clamp_val = 0x20280a02u - sub;
    return (io | clamp_val) & (clamp_val - sub);
}

vec3 rgb_out_impl(uint raw) {
    uint rgb = ((raw >> 5u) & 0x00ff0000u) | ((raw >> 3u) & 0x0000ff00u) | ((raw >> 1u) & 0x000000ffu);
    return vec3(
        float((rgb >> 16u) & 0xffu),
        float((rgb >> 8u) & 0xffu),
        float(rgb & 0xffu)
    ) / 255.0;
}

void main(void) {
    ivec2 out_pos = ivec2(gl_FragCoord.xy);

    if (ntsc_enabled) {
        int chunk = out_pos.x / 7;
        int sample = out_pos.x - chunk * 7;
        int base = chunk * 3;
        int phase_row = (out_pos.y % 3) * NTSC_ENTRY_STRIDE;

        uint sum =
            ntsc_entry(palette_index(ivec2(base + ntsc_source_offset(sample, 0), out_pos.y)), phase_row + ntsc_row_offset(sample, 0)) +
            ntsc_entry(palette_index(ivec2(base + ntsc_source_offset(sample, 1), out_pos.y)), phase_row + ntsc_row_offset(sample, 1)) +
            ntsc_entry(palette_index(ivec2(base + ntsc_source_offset(sample, 2), out_pos.y)), phase_row + ntsc_row_offset(sample, 2)) +
            ntsc_entry(palette_index(ivec2(base + ntsc_source_offset(sample, 3), out_pos.y)), phase_row + ntsc_row_offset(sample, 3)) +
            ntsc_entry(palette_index(ivec2(base + ntsc_source_offset(sample, 4), out_pos.y)), phase_row + ntsc_row_offset(sample, 4)) +
            ntsc_entry(palette_index(ivec2(base + ntsc_source_offset(sample, 5), out_pos.y)), phase_row + ntsc_row_offset(sample, 5));

        frag_color = vec4(rgb_out_impl(clamp_impl(sum)), 1.0);
    } else {
        uint idx = palette_index(ivec2(out_pos.x, int(source_size.y) - 1 - out_pos.y));
        frag_color = vec4(palette_color(idx), 1.0);
    }
}
