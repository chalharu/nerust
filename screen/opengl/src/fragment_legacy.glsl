#version 120

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
varying vec2 vuv;

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
    return int(floor(texture2D(frame_texture, uv).r * 255.0 + 0.5));
}

vec3 palette_color(int index) {
    vec2 uv = center_uv(vec2(64.0, 1.0), float(index), 0.0);
    return texture2D(palette_texture, uv).rgb;
}

vec3 ntsc_color(int output_x, int output_y) {
    int chunk = output_x / 7;
    int sample = output_x - chunk * 7;
    int base = chunk * 3;
    int phase = output_y % 3;
    int prev0 = palette_index(base - 2, output_y);
    int prev1 = palette_index(base - 1, output_y);
    int prev2 = palette_index(base, output_y);
    int curr0 = palette_index(base + 1, output_y);
    int curr1 = palette_index(base + 2, output_y);
    int curr2 = palette_index(base + 3, output_y);
    float row0 = float(phase * NTSC_ENTRY_STRIDE + sample);
    float row1 = float(phase * NTSC_ENTRY_STRIDE + ((sample + 12) % 7 + 14));
    float row2 = float(phase * NTSC_ENTRY_STRIDE + ((sample + 10) % 7 + 28));
    float row3 = float(phase * NTSC_ENTRY_STRIDE + ((sample + 7) % 14));
    float row4 = float(phase * NTSC_ENTRY_STRIDE + ((sample + 5) % 7 + 21));
    float row5 = float(phase * NTSC_ENTRY_STRIDE + ((sample + 3) % 7 + 35));
    vec4 p0 = texture2D(ntsc_primary_texture, center_uv(vec2(64.0, 126.0), float(curr0), row0));
    vec4 p1 = texture2D(ntsc_primary_texture, center_uv(vec2(64.0, 126.0), float(curr1), row1));
    vec4 p2 = texture2D(ntsc_primary_texture, center_uv(vec2(64.0, 126.0), float(curr2), row2));
    vec4 p3 = texture2D(ntsc_primary_texture, center_uv(vec2(64.0, 126.0), float(prev0), row3));
    vec4 p4 = texture2D(ntsc_primary_texture, center_uv(vec2(64.0, 126.0), float(prev1), row4));
    vec4 p5 = texture2D(ntsc_primary_texture, center_uv(vec2(64.0, 126.0), float(prev2), row5));
    vec4 s0 = texture2D(ntsc_secondary_texture, center_uv(vec2(64.0, 126.0), float(curr0), row0));
    vec4 s1 = texture2D(ntsc_secondary_texture, center_uv(vec2(64.0, 126.0), float(curr1), row1));
    vec4 s2 = texture2D(ntsc_secondary_texture, center_uv(vec2(64.0, 126.0), float(curr2), row2));
    vec4 s3 = texture2D(ntsc_secondary_texture, center_uv(vec2(64.0, 126.0), float(prev0), row3));
    vec4 s4 = texture2D(ntsc_secondary_texture, center_uv(vec2(64.0, 126.0), float(prev1), row4));
    vec4 s5 = texture2D(ntsc_secondary_texture, center_uv(vec2(64.0, 126.0), float(prev2), row5));
    vec3 sum = vec3(
        float(decode_u16(texel_u8(p0, 0), texel_u8(p0, 1)) + decode_u16(texel_u8(p1, 0), texel_u8(p1, 1)) + decode_u16(texel_u8(p2, 0), texel_u8(p2, 1)) + decode_u16(texel_u8(p3, 0), texel_u8(p3, 1)) + decode_u16(texel_u8(p4, 0), texel_u8(p4, 1)) + decode_u16(texel_u8(p5, 0), texel_u8(p5, 1))),
        float(decode_u16(texel_u8(p0, 2), texel_u8(p0, 3)) + decode_u16(texel_u8(p1, 2), texel_u8(p1, 3)) + decode_u16(texel_u8(p2, 2), texel_u8(p2, 3)) + decode_u16(texel_u8(p3, 2), texel_u8(p3, 3)) + decode_u16(texel_u8(p4, 2), texel_u8(p4, 3)) + decode_u16(texel_u8(p5, 2), texel_u8(p5, 3))),
        float(decode_u16(texel_u8(s0, 0), texel_u8(s0, 1)) + decode_u16(texel_u8(s1, 0), texel_u8(s1, 1)) + decode_u16(texel_u8(s2, 0), texel_u8(s2, 1)) + decode_u16(texel_u8(s3, 0), texel_u8(s3, 1)) + decode_u16(texel_u8(s4, 0), texel_u8(s4, 1)) + decode_u16(texel_u8(s5, 0), texel_u8(s5, 1)))
    );
    vec3 clamped = clamp(sum - vec3(float(NTSC_CHANNEL_BIAS)), vec3(0.0), vec3(255.0));
    return clamped / 255.0;
}

void main(void){
    int output_x = min(int(floor(vuv.x * float(output_width))), output_width - 1);
    int output_y = min(int(floor(vuv.y * float(output_height))), output_height - 1);
    vec3 color = filter_mode == 0
        ? palette_color(palette_index(min(output_x, source_width - 1), min(output_y, source_height - 1)))
        : ntsc_color(output_x, output_y);
    gl_FragColor = vec4(color, 1.0);
}
