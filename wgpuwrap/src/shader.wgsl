struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(2.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

@group(0) @binding(0)
var frame_texture: texture_2d<f32>;

@group(0) @binding(1)
var palette_texture: texture_2d<f32>;

@group(0) @binding(2)
var ntsc_primary_texture: texture_2d<f32>;

@group(0) @binding(3)
var ntsc_secondary_texture: texture_2d<f32>;

struct FilterUniforms {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
    filter_mode: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
};

@group(0) @binding(4)
var<uniform> uniforms: FilterUniforms;

const BLACK_INDEX: i32 = 15;
const NTSC_ENTRY_STRIDE: i32 = 42;
const NTSC_CHANNEL_BIAS: i32 = 512;

fn decode_u8(value: f32) -> u32 {
    return u32(round(clamp(value, 0.0, 1.0) * 255.0));
}

fn decode_u16(high: u32, low: u32) -> i32 {
    return i32((high << 8u) | low) - 32768;
}

fn palette_index(x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= i32(uniforms.source_width) || y >= i32(uniforms.source_height) {
        return BLACK_INDEX;
    }
    return i32(round(textureLoad(frame_texture, vec2<i32>(x, y), 0).r * 255.0));
}

fn palette_color(index: i32) -> vec3<f32> {
    return textureLoad(palette_texture, vec2<i32>(index, 0), 0).rgb;
}

fn ntsc_entry(color: i32, phase: i32, offset: i32) -> vec3<i32> {
    let row = phase * NTSC_ENTRY_STRIDE + offset;
    let primary = textureLoad(ntsc_primary_texture, vec2<i32>(color, row), 0);
    let secondary = textureLoad(ntsc_secondary_texture, vec2<i32>(color, row), 0);
    return vec3<i32>(
        decode_u16(decode_u8(primary.r), decode_u8(primary.g)),
        decode_u16(decode_u8(primary.b), decode_u8(primary.a)),
        decode_u16(decode_u8(secondary.r), decode_u8(secondary.g)),
    );
}

fn ntsc_color(output_x: i32, output_y: i32) -> vec3<f32> {
    let chunk = output_x / 7;
    let sample = output_x - chunk * 7;
    let base = chunk * 3;
    let phase = output_y % 3;
    let previous = vec3<i32>(
        palette_index(base - 2, output_y),
        palette_index(base - 1, output_y),
        palette_index(base, output_y),
    );
    let current = vec3<i32>(
        palette_index(base + 1, output_y),
        palette_index(base + 2, output_y),
        palette_index(base + 3, output_y),
    );
    let sum = ntsc_entry(current.x, phase, sample)
        + ntsc_entry(current.y, phase, (sample + 12) % 7 + 14)
        + ntsc_entry(current.z, phase, (sample + 10) % 7 + 28)
        + ntsc_entry(previous.x, phase, (sample + 7) % 14)
        + ntsc_entry(previous.y, phase, (sample + 5) % 7 + 21)
        + ntsc_entry(previous.z, phase, (sample + 3) % 7 + 35);
    let clamped = clamp(
        sum - vec3<i32>(NTSC_CHANNEL_BIAS),
        vec3<i32>(0),
        vec3<i32>(255),
    );
    return vec3<f32>(clamped) / 255.0;
}

fn srgb_channel_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        return channel / 12.92;
    }
    return pow((channel + 0.055) / 1.055, 2.4);
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_channel_to_linear(color.r),
        srgb_channel_to_linear(color.g),
        srgb_channel_to_linear(color.b),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let output_x = i32(
        min(floor(input.uv.x * f32(uniforms.output_width)), f32(uniforms.output_width - 1u)),
    );
    let output_y = i32(
        min(floor(input.uv.y * f32(uniforms.output_height)), f32(uniforms.output_height - 1u)),
    );
    let srgb_color = if uniforms.filter_mode == 0u {
        let source_x = min(output_x, i32(uniforms.source_width) - 1);
        let source_y = min(output_y, i32(uniforms.source_height) - 1);
        palette_color(palette_index(source_x, source_y))
    } else {
        ntsc_color(output_x, output_y)
    };
    return vec4<f32>(srgb_to_linear(srgb_color), 1.0);
}
