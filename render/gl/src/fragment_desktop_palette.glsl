uniform usampler2D palette_texture;

vec3 decoded_rgb_for_output(ivec2 output_pos) {
    ivec2 source_pos = palette_source_coords(output_pos);
    uint index = texelFetch(frame_texture, source_pos, 0).r;
    return vec3(texelFetch(palette_texture, ivec2(int(index), 0), 0).rgb) / 255.0;
}
