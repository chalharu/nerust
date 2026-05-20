struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const FULLSCREEN_POSITIONS: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -3.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(3.0, 1.0),
);

const FULLSCREEN_UVS: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
    vec2<f32>(0.0, 2.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(2.0, 0.0),
);

const NTSC_ROW_OFFSETS: array<array<i32, 6>, 7> = array<array<i32, 6>, 7>(
    array<i32, 6>(0, 19, 31, 7, 26, 38),
    array<i32, 6>(1, 20, 32, 8, 27, 39),
    array<i32, 6>(2, 14, 33, 9, 21, 40),
    array<i32, 6>(3, 15, 34, 10, 22, 41),
    array<i32, 6>(4, 16, 28, 11, 23, 35),
    array<i32, 6>(5, 17, 29, 12, 24, 36),
    array<i32, 6>(6, 18, 30, 13, 25, 37),
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let vertex = i32(vertex_index);
    var output: VertexOutput;
    output.position = vec4<f32>(FULLSCREEN_POSITIONS[vertex], 0.0, 1.0);
    output.uv = FULLSCREEN_UVS[vertex];
    return output;
}

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
var srgb_lut_texture: texture_1d<f32>;

const BLACK_INDEX: u32 = 15u;
const NTSC_ENTRY_STRIDE: i32 = 42;
const NTSC_CLAMP_MASK: u32 = 0x300c03u;
const NTSC_CLAMP_ADD: u32 = 0x20280a02u;

fn output_coords(uv: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(
        i32(min(floor(uv.x * f32(uniforms.output_width)), f32(uniforms.output_width - 1u))),
        i32(min(floor(uv.y * f32(uniforms.output_height)), f32(uniforms.output_height - 1u))),
    );
}

fn palette_index(x: i32, y: i32) -> u32 {
    if x < 0 || y < 0 || x >= i32(uniforms.source_width) || y >= i32(uniforms.source_height) {
        return BLACK_INDEX;
    }
    return textureLoad(frame_texture, vec2<i32>(x, y), 0).r;
}

fn palette_color(index: u32) -> vec3<u32> {
    return textureLoad(palette_texture, vec2<i32>(i32(index), 0), 0).rgb;
}

fn ntsc_entry(color: u32, row: i32) -> u32 {
    return textureLoad(ntsc_texture, vec2<i32>(i32(color), row), 0).r;
}

fn clamp_impl(io: u32) -> u32 {
    let sub = (io >> 9u) & NTSC_CLAMP_MASK;
    let clamp = NTSC_CLAMP_ADD - sub;
    return (io | clamp) & (clamp - sub);
}

fn rgb_out_impl(raw: u32) -> vec3<u32> {
    let rgb = ((raw >> 5u) & 0x00ff0000u) | ((raw >> 3u) & 0x0000ff00u) | ((raw >> 1u) & 0x000000ffu);
    return vec3<u32>((rgb >> 16u) & 0xffu, (rgb >> 8u) & 0xffu, rgb & 0xffu);
}

fn ntsc_color(output_x: i32, output_y: i32) -> vec3<u32> {
    let chunk = output_x / 7;
    let sample = output_x - chunk * 7;
    let base = chunk * 3;
    let phase_row = (output_y % 3) * NTSC_ENTRY_STRIDE;
    let colors = array<u32, 6>(
        palette_index(base + 1, output_y),
        palette_index(base + 2, output_y),
        palette_index(base + 3, output_y),
        palette_index(base - 2, output_y),
        palette_index(base - 1, output_y),
        palette_index(base, output_y),
    );
    let offsets = NTSC_ROW_OFFSETS[u32(sample)];
    let entries = array<u32, 6>(
        ntsc_entry(colors[0], phase_row + offsets[0]),
        ntsc_entry(colors[1], phase_row + offsets[1]),
        ntsc_entry(colors[2], phase_row + offsets[2]),
        ntsc_entry(colors[3], phase_row + offsets[3]),
        ntsc_entry(colors[4], phase_row + offsets[4]),
        ntsc_entry(colors[5], phase_row + offsets[5]),
    );
    let sum = entries[0] + entries[1] + entries[2] + entries[3] + entries[4] + entries[5];
    return rgb_out_impl(clamp_impl(sum));
}

fn srgb_to_linear(color: vec3<u32>) -> vec3<f32> {
    return vec3<f32>(
        textureLoad(srgb_lut_texture, i32(color.r), 0).r,
        textureLoad(srgb_lut_texture, i32(color.g), 0).r,
        textureLoad(srgb_lut_texture, i32(color.b), 0).r,
    );
}

fn unorm_to_vec4(color: vec3<u32>) -> vec4<f32> {
    return vec4<f32>(vec3<f32>(color) / 255.0, 1.0);
}

fn srgb_to_vec4(color: vec3<u32>) -> vec4<f32> {
    return vec4<f32>(srgb_to_linear(color), 1.0);
}

@fragment
fn fs_palette_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    let coords = output_coords(input.uv);
    let source_x = min(coords.x, i32(uniforms.source_width) - 1);
    let source_y = min(coords.y, i32(uniforms.source_height) - 1);
    let color = palette_color(palette_index(source_x, source_y));
    return unorm_to_vec4(color);
}

@fragment
fn fs_palette_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    let coords = output_coords(input.uv);
    let source_x = min(coords.x, i32(uniforms.source_width) - 1);
    let source_y = min(coords.y, i32(uniforms.source_height) - 1);
    let color = palette_color(palette_index(source_x, source_y));
    return srgb_to_vec4(color);
}

@fragment
fn fs_ntsc_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    let coords = output_coords(input.uv);
    return unorm_to_vec4(ntsc_color(coords.x, coords.y));
}

@fragment
fn fs_ntsc_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    let coords = output_coords(input.uv);
    return srgb_to_vec4(ntsc_color(coords.x, coords.y));
}
