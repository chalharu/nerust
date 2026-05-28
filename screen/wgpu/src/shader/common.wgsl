@group(0) @binding(0)
var frame_texture: texture_2d<u32>;

@group(0) @binding(1)
var palette_texture: texture_2d<u32>;

@group(0) @binding(2)
var ntsc_texture: texture_2d<u32>;

struct FilterUniforms {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
};

@group(0) @binding(3)
var<uniform> uniforms: FilterUniforms;

@group(0) @binding(4)
var srgb_lut_texture: texture_2d<f32>;

const BLACK_INDEX: u32 = 15u;

fn output_coords(uv: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(
        i32(min(floor(uv.x * f32(uniforms.output_width)), f32(uniforms.output_width - 1u))),
        i32(min(floor(uv.y * f32(uniforms.output_height)), f32(uniforms.output_height - 1u))),
    );
}

fn palette_source_coords(output: vec2<i32>) -> vec2<i32> {
    return vec2<i32>(
        min(output.x, i32(uniforms.source_width) - 1),
        min(output.y, i32(uniforms.source_height) - 1),
    );
}

fn direct_source_coords(output: vec2<i32>) -> vec2<i32> {
    let output_width = i32(max(uniforms.output_width, 1u));
    let output_height = i32(max(uniforms.output_height, 1u));
    return vec2<i32>(
        min((output.x * i32(uniforms.source_width)) / output_width, i32(uniforms.source_width) - 1),
        min((output.y * i32(uniforms.source_height)) / output_height, i32(uniforms.source_height) - 1),
    );
}

fn palette_index(x: i32, y: i32) -> u32 {
    if x < 0 || y < 0 || x >= i32(uniforms.source_width) || y >= i32(uniforms.source_height) {
        return BLACK_INDEX;
    }
    return textureLoad(frame_texture, vec2<i32>(x, y), 0).r;
}

fn direct_rgb_for_output(output: vec2<i32>) -> vec3<u32> {
    return textureLoad(frame_texture, direct_source_coords(output), 0).rgb;
}

fn srgb_to_linear(color: vec3<u32>) -> vec3<f32> {
    return vec3<f32>(
        textureLoad(srgb_lut_texture, vec2<i32>(i32(color.r), 0), 0).r,
        textureLoad(srgb_lut_texture, vec2<i32>(i32(color.g), 0), 0).r,
        textureLoad(srgb_lut_texture, vec2<i32>(i32(color.b), 0), 0).r,
    );
}

fn unorm_to_vec4(color: vec3<u32>) -> vec4<f32> {
    return vec4<f32>(vec3<f32>(color) / 255.0, 1.0);
}

fn srgb_to_vec4(color: vec3<u32>) -> vec4<f32> {
    return vec4<f32>(srgb_to_linear(color), 1.0);
}
