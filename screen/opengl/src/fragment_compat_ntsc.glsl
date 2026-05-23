uniform sampler2D ntsc_primary_texture;
uniform sampler2D ntsc_secondary_texture;

const int BLACK_INDEX = 15;
const int NTSC_ENTRY_STRIDE = 42;
const int NTSC_CHANNEL_BIAS = 512;

NERUST_MEDIUMP float texel_u8(vec4 texel, int channel) {
    return floor(clamp(texel[channel], 0.0, 1.0) * 255.0 + 0.5);
}

int decode_u16(NERUST_MEDIUMP float high, NERUST_MEDIUMP float low) {
    return int(high) * 256 + int(low) - 32768;
}

int palette_index(int x, int y) {
    if (x < 0 || y < 0 || x >= source_width || y >= source_height) {
        return BLACK_INDEX;
    }
    NERUST_MEDIUMP vec2 uv = vec2(
        (float(x) + 0.5) / float(source_width) * frame_uv_size.x,
        (float(y) + 0.5) / float(source_height) * frame_uv_size.y
    );
    return int(floor(texture2D(frame_texture, uv).r * 255.0 + 0.5));
}

NERUST_MEDIUMP vec3 decoded_rgb_for_output(ivec2 output_pos) {
    int chunk = output_pos.x / 7;
    int sample = output_pos.x - chunk * 7;
    int base = chunk * 3;
    int phase = output_pos.y % 3;
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
    vec3 clamped = clamp(
        sum - vec3(float(NTSC_CHANNEL_BIAS)),
        vec3(0.0),
        vec3(255.0)
    );
    return clamped / 255.0;
}
