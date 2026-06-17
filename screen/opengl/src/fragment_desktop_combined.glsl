uniform sampler2D frame_texture;
uniform sampler2D palette_texture;
uniform vec2 source_size;
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

void main(void) {
    ivec2 out_pos = ivec2(gl_FragCoord.xy);
    uint idx = palette_index(out_pos);
    frag_color = vec4(palette_color(idx), 1.0);
}
