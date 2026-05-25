@fragment
fn fs_palette_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    return unorm_to_vec4(palette_rgb_for_output(output_coords(input.uv)));
}

@fragment
fn fs_direct_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    return unorm_to_vec4(direct_rgb_for_output(output_coords(input.uv)));
}

@fragment
fn fs_palette_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    return srgb_to_vec4(palette_rgb_for_output(output_coords(input.uv)));
}

@fragment
fn fs_direct_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    return srgb_to_vec4(direct_rgb_for_output(output_coords(input.uv)));
}

@fragment
fn fs_ntsc_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    return unorm_to_vec4(ntsc_rgb_for_output(output_coords(input.uv)));
}

@fragment
fn fs_ntsc_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    return srgb_to_vec4(ntsc_rgb_for_output(output_coords(input.uv)));
}
