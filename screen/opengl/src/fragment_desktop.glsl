#version 150

uniform sampler2D frame_texture;
uniform sampler2D palette_texture;
uniform usampler2D ntsc_texture;
uniform vec2 frame_uv_size;
uniform int source_width;
uniform int source_height;
uniform int output_width;
uniform int output_height;
uniform int filter_mode;
in vec2 vuv;
out vec4 frag_color;

const int BLACK_INDEX = 15;
const int NTSC_ENTRY_STRIDE = 42;
const uint NTSC_CLAMP_MASK = 0x300c03u;
const uint NTSC_CLAMP_ADD = 0x20280a02u;

vec2 center_uv(vec2 texture_size, float x, float y) {
    return vec2((x + 0.5) / texture_size.x, (y + 0.5) / texture_size.y);
}

int palette_index(int x, int y) {
    if (x < 0 || y < 0 || x >= source_width || y >= source_height) {
        return BLACK_INDEX;
    }
    vec2 uv = vec2(
        (float(x) + 0.5) / float(source_width) * frame_uv_size.x,
        (float(y) + 0.5) / float(source_height) * frame_uv_size.y
    );
    return int(floor(texture(frame_texture, uv).r * 255.0 + 0.5));
}

vec3 palette_color(int index) {
    vec2 uv = center_uv(vec2(64.0, 1.0), float(index), 0.0);
    return texture(palette_texture, uv).rgb;
}

uint ntsc_entry(int color, int phase, int offset) {
    int row = phase * NTSC_ENTRY_STRIDE + offset;
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
    int phase = output_y % 3;
    ivec3 previous = ivec3(
        palette_index(base - 2, output_y),
        palette_index(base - 1, output_y),
        palette_index(base, output_y)
    );
    ivec3 current = ivec3(
        palette_index(base + 1, output_y),
        palette_index(base + 2, output_y),
        palette_index(base + 3, output_y)
    );
    uint sum = ntsc_entry(current.x, phase, sample)
        + ntsc_entry(current.y, phase, (sample + 12) % 7 + 14)
        + ntsc_entry(current.z, phase, (sample + 10) % 7 + 28)
        + ntsc_entry(previous.x, phase, (sample + 7) % 14)
        + ntsc_entry(previous.y, phase, (sample + 5) % 7 + 21)
        + ntsc_entry(previous.z, phase, (sample + 3) % 7 + 35);
    return rgb_out_impl(clamp_impl(sum));
}

void main(void){
    int output_x = min(int(floor(vuv.x * float(output_width))), output_width - 1);
    int output_y = min(int(floor(vuv.y * float(output_height))), output_height - 1);
    vec3 color = filter_mode == 0
        ? palette_color(palette_index(min(output_x, source_width - 1), min(output_y, source_height - 1)))
        : ntsc_color(output_x, output_y);
    frag_color = vec4(color, 1.0);
}
