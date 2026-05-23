uniform sampler2D palette_texture;

int palette_index_at_source(ivec2 source_pos) {
    NERUST_MEDIUMP vec2 uv = vec2(
        (float(source_pos.x) + 0.5) / float(source_width) * frame_uv_size.x,
        (float(source_pos.y) + 0.5) / float(source_height) * frame_uv_size.y
    );
    return int(floor(texture2D(frame_texture, uv).r * 255.0 + 0.5));
}

NERUST_MEDIUMP vec3 decoded_rgb_for_output(ivec2 output_pos) {
    ivec2 source_pos = palette_source_coords(output_pos);
    return texture2D(
        palette_texture,
        center_uv(vec2(64.0, 1.0), float(palette_index_at_source(source_pos)), 0.0)
    ).rgb;
}
