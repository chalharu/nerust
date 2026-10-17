struct OsdVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle like the game pass, but UVs span 0..1 across
// the viewport so the cropped layer maps exactly onto its rect.
@vertex
fn vs_osd(@builtin(vertex_index) vertex_index: u32) -> OsdVertexOutput {
    var pos: vec2<f32>;
    if vertex_index == 0u {
        pos = vec2<f32>(-1.0, -3.0);
    } else if vertex_index == 1u {
        pos = vec2<f32>(-1.0, 1.0);
    } else {
        pos = vec2<f32>(3.0, 1.0);
    }
    var output: OsdVertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    output.uv = vec2<f32>((pos.x + 1.0) * 0.5, (1.0 - pos.y) * 0.5);
    return output;
}

@group(0) @binding(0) var osd_texture: texture_2d<f32>;
@group(0) @binding(1) var osd_sampler: sampler;

@fragment
fn fs_osd(input: OsdVertexOutput) -> @location(0) vec4<f32> {
    return textureSample(osd_texture, osd_sampler, input.uv);
}
