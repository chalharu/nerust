uniform sampler2D frame_texture;
uniform NERUST_MEDIUMP vec2 frame_uv_size;
uniform int source_width;
uniform int source_height;
uniform int output_width;
uniform int output_height;
varying NERUST_MEDIUMP vec2 vuv;

NERUST_MEDIUMP vec2 center_uv(
    NERUST_MEDIUMP vec2 texture_size,
    NERUST_MEDIUMP float x,
    NERUST_MEDIUMP float y
) {
    return vec2((x + 0.5) / texture_size.x, (y + 0.5) / texture_size.y);
}

ivec2 output_coords() {
    return ivec2(
        min(int(floor(vuv.x * float(output_width))), output_width - 1),
        min(int(floor(vuv.y * float(output_height))), output_height - 1)
    );
}

ivec2 palette_source_coords(ivec2 output_pos) {
    return ivec2(
        min(output_pos.x, source_width - 1),
        min(output_pos.y, source_height - 1)
    );
}
