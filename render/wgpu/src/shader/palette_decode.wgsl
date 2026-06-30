fn palette_color(index: u32) -> vec3<u32> {
    return textureLoad(palette_texture, vec2<i32>(i32(index), 0), 0).rgb;
}

fn palette_rgb_for_output(output: vec2<i32>) -> vec3<u32> {
    let source = palette_source_coords(output);
    return palette_color(palette_index(source.x, source.y));
}
