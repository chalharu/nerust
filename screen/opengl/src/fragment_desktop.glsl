#version 150

uniform sampler2D frame_texture;
uniform sampler2D palette_texture;
uniform sampler2D ntsc_primary_texture;
uniform sampler2D ntsc_secondary_texture;
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
const int NTSC_CHANNEL_BIAS = 512;

float texel_u8(vec4 texel, int channel) {
    return floor(clamp(texel[channel], 0.0, 1.0) * 255.0 + 0.5);
}

int decode_u16(float high, float low) {
    return int(high) * 256 + int(low) - 32768;
}

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

ivec3 ntsc_entry(int color, int phase, int offset) {
    float row = float(phase * NTSC_ENTRY_STRIDE + offset);
    vec2 uv = center_uv(vec2(64.0, 126.0), float(color), row);
    vec4 primary = texture(ntsc_primary_texture, uv);
    vec4 secondary = texture(ntsc_secondary_texture, uv);
    return ivec3(
        decode_u16(texel_u8(primary, 0), texel_u8(primary, 1)),
        decode_u16(texel_u8(primary, 2), texel_u8(primary, 3)),
        decode_u16(texel_u8(secondary, 0), texel_u8(secondary, 1))
    );
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
    ivec3 sum = ntsc_entry(current.x, phase, sample)
        + ntsc_entry(current.y, phase, (sample + 12) % 7 + 14)
        + ntsc_entry(current.z, phase, (sample + 10) % 7 + 28)
        + ntsc_entry(previous.x, phase, (sample + 7) % 14)
        + ntsc_entry(previous.y, phase, (sample + 5) % 7 + 21)
        + ntsc_entry(previous.z, phase, (sample + 3) % 7 + 35);
    vec3 clamped = clamp(vec3(sum - ivec3(NTSC_CHANNEL_BIAS)), vec3(0.0), vec3(255.0));
    return clamped / 255.0;
}

void main(void){
    int output_x = min(int(floor(vuv.x * float(output_width))), output_width - 1);
    int output_y = min(int(floor(vuv.y * float(output_height))), output_height - 1);
    vec3 color = filter_mode == 0
        ? palette_color(palette_index(min(output_x, source_width - 1), min(output_y, source_height - 1)))
        : ntsc_color(output_x, output_y);
    frag_color = vec4(color, 1.0);
}
