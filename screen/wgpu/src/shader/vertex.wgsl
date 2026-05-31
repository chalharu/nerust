struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

fn fullscreen_position(vertex: u32) -> vec2<f32> {
    if vertex == 0u {
        return vec2<f32>(-1.0, -3.0);
    }
    if vertex == 1u {
        return vec2<f32>(-1.0, 1.0);
    }
    return vec2<f32>(3.0, 1.0);
}

fn fullscreen_uv(vertex: u32) -> vec2<f32> {
    if vertex == 0u {
        return vec2<f32>(0.0, 2.0);
    }
    if vertex == 1u {
        return vec2<f32>(0.0, 0.0);
    }
    return vec2<f32>(2.0, 0.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(fullscreen_position(vertex_index), 0.0, 1.0);
    output.uv = fullscreen_uv(vertex_index);
    return output;
}
