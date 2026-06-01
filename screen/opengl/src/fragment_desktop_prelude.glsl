uniform usampler2D frame_texture;
uniform int source_width;
uniform int source_height;
uniform int output_width;
uniform int output_height;
in vec2 vuv;
out vec4 frag_color;

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
