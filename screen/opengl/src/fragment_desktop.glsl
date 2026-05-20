#version 150

uniform usampler2D frame_texture;
#ifdef NERUST_FILTER_PALETTE
uniform usampler2D palette_texture;
#else
uniform usampler2D ntsc_texture;
#endif
uniform int source_width;
uniform int source_height;
uniform int output_width;
uniform int output_height;
in vec2 vuv;
out vec4 frag_color;

#ifdef NERUST_FILTER_PALETTE
vec3 palette_color(int index) {
    return vec3(texelFetch(palette_texture, ivec2(index, 0), 0).rgb) / 255.0;
}
#else
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

vec3 ntsc_color(int output_x, int output_y) {
    int chunk = output_x / 7;
    int sample = output_x - chunk * 7;
    int base = chunk * 3;
    int phase_row = (output_y % 3) * NTSC_ENTRY_STRIDE;
    int curr0 = palette_index(base + 1, output_y);
    int curr1 = palette_index(base + 2, output_y);
    int curr2 = palette_index(base + 3, output_y);
    int prev0 = palette_index(base - 2, output_y);
    int prev1 = palette_index(base - 1, output_y);
    int prev2 = palette_index(base, output_y);
    uint entry0 = ntsc_entry(curr0, phase_row + sample);
    uint entry1 = ntsc_entry(curr1, phase_row + ((sample + 12) % 7 + 14));
    uint entry2 = ntsc_entry(curr2, phase_row + ((sample + 10) % 7 + 28));
    uint entry3 = ntsc_entry(prev0, phase_row + ((sample + 7) % 14));
    uint entry4 = ntsc_entry(prev1, phase_row + ((sample + 5) % 7 + 21));
    uint entry5 = ntsc_entry(prev2, phase_row + ((sample + 3) % 7 + 35));
    return rgb_out_impl(clamp_impl(entry0 + entry1 + entry2 + entry3 + entry4 + entry5));
}
#endif

void main(void) {
    int output_x = min(int(floor(vuv.x * float(output_width))), output_width - 1);
    int output_y = min(int(floor(vuv.y * float(output_height))), output_height - 1);
#ifdef NERUST_FILTER_PALETTE
    int source_x = min(output_x, source_width - 1);
    int source_y = min(output_y, source_height - 1);
    int index = int(texelFetch(frame_texture, ivec2(source_x, source_y), 0).r);
    frag_color = vec4(palette_color(index), 1.0);
#else
    frag_color = vec4(ntsc_color(output_x, output_y), 1.0);
#endif
}
