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

const SRGB_TO_LINEAR: array<f32, 256> = array<f32, 256>(
    0.0000000000,
    0.0003035270,
    0.0006070540,
    0.0009105810,
    0.0012141079,
    0.0015176349,
    0.0018211619,
    0.0021246889,
    0.0024282159,
    0.0027317429,
    0.0030352698,
    0.0033465358,
    0.0036765073,
    0.0040247170,
    0.0043914420,
    0.0047769535,
    0.0051815167,
    0.0056053916,
    0.0060488330,
    0.0065120908,
    0.0069954102,
    0.0074990320,
    0.0080231930,
    0.0085681256,
    0.0091340587,
    0.0097212173,
    0.0103298230,
    0.0109600940,
    0.0116122452,
    0.0122864884,
    0.0129830323,
    0.0137020830,
    0.0144438436,
    0.0152085144,
    0.0159962934,
    0.0168073758,
    0.0176419545,
    0.0185002201,
    0.0193823610,
    0.0202885631,
    0.0212190104,
    0.0221738848,
    0.0231533662,
    0.0241576324,
    0.0251868596,
    0.0262412219,
    0.0273208916,
    0.0284260395,
    0.0295568344,
    0.0307134437,
    0.0318960331,
    0.0331047666,
    0.0343398068,
    0.0356013149,
    0.0368894504,
    0.0382043716,
    0.0395462353,
    0.0409151969,
    0.0423114106,
    0.0437350293,
    0.0451862044,
    0.0466650863,
    0.0481718242,
    0.0497065660,
    0.0512694584,
    0.0528606470,
    0.0544802764,
    0.0561284900,
    0.0578054302,
    0.0595112382,
    0.0612460542,
    0.0630100177,
    0.0648032667,
    0.0666259386,
    0.0684781698,
    0.0703600957,
    0.0722718507,
    0.0742135684,
    0.0761853815,
    0.0781874218,
    0.0802198203,
    0.0822827071,
    0.0843762115,
    0.0865004620,
    0.0886555863,
    0.0908417112,
    0.0930589628,
    0.0953074666,
    0.0975873471,
    0.0998987282,
    0.1022417331,
    0.1046164841,
    0.1070231030,
    0.1094617108,
    0.1119324278,
    0.1144353738,
    0.1169706678,
    0.1195384280,
    0.1221387722,
    0.1247718176,
    0.1274376804,
    0.1301364767,
    0.1328683216,
    0.1356333297,
    0.1384316150,
    0.1412632911,
    0.1441284709,
    0.1470272665,
    0.1499597898,
    0.1529261520,
    0.1559264637,
    0.1589608351,
    0.1620293756,
    0.1651321945,
    0.1682694002,
    0.1714411007,
    0.1746474037,
    0.1778884160,
    0.1811642442,
    0.1844749945,
    0.1878207723,
    0.1912016827,
    0.1946178304,
    0.1980693196,
    0.2015562538,
    0.2050787364,
    0.2086368701,
    0.2122307574,
    0.2158605001,
    0.2195261997,
    0.2232279573,
    0.2269658735,
    0.2307400485,
    0.2345505822,
    0.2383975738,
    0.2422811225,
    0.2462013267,
    0.2501582847,
    0.2541520943,
    0.2581828529,
    0.2622506575,
    0.2663556048,
    0.2704977910,
    0.2746773121,
    0.2788942635,
    0.2831487404,
    0.2874408377,
    0.2917706498,
    0.2961382708,
    0.3005437944,
    0.3049873141,
    0.3094689228,
    0.3139887134,
    0.3185467781,
    0.3231432091,
    0.3277780981,
    0.3324515363,
    0.3371636150,
    0.3419144249,
    0.3467040564,
    0.3515325995,
    0.3564001441,
    0.3613067798,
    0.3662525956,
    0.3712376805,
    0.3762621230,
    0.3813260114,
    0.3864294338,
    0.3915724777,
    0.3967552307,
    0.4019777798,
    0.4072402119,
    0.4125426135,
    0.4178850708,
    0.4232676700,
    0.4286904966,
    0.4341536362,
    0.4396571738,
    0.4452011945,
    0.4507857828,
    0.4564110232,
    0.4620769997,
    0.4677837961,
    0.4735314961,
    0.4793201831,
    0.4851499401,
    0.4910208498,
    0.4969329951,
    0.5028864580,
    0.5088813209,
    0.5149176654,
    0.5209955732,
    0.5271151257,
    0.5332764040,
    0.5394794890,
    0.5457244614,
    0.5520114015,
    0.5583403896,
    0.5647115057,
    0.5711248295,
    0.5775804404,
    0.5840784179,
    0.5906188409,
    0.5972017884,
    0.6038273389,
    0.6104955708,
    0.6172065624,
    0.6239603917,
    0.6307571363,
    0.6375968740,
    0.6444796820,
    0.6514056374,
    0.6583748173,
    0.6653872983,
    0.6724431570,
    0.6795424696,
    0.6866853124,
    0.6938717613,
    0.7011018919,
    0.7083757799,
    0.7156935005,
    0.7230551289,
    0.7304607401,
    0.7379104088,
    0.7454042095,
    0.7529422168,
    0.7605245047,
    0.7681511472,
    0.7758222183,
    0.7835377915,
    0.7912979403,
    0.7991027380,
    0.8069522577,
    0.8148465722,
    0.8227857544,
    0.8307698768,
    0.8387990117,
    0.8468732315,
    0.8549926081,
    0.8631572135,
    0.8713671192,
    0.8796223969,
    0.8879231179,
    0.8962693534,
    0.9046611744,
    0.9130986518,
    0.9215818563,
    0.9301108584,
    0.9386857285,
    0.9473065367,
    0.9559733532,
    0.9646862479,
    0.9734452904,
    0.9822505503,
    0.9911020971,
    1.0000000000
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
    let packed = textureLoad(ntsc_texture, vec2<i32>(i32(color), row), 0);
    return (packed.r << 24u) | (packed.g << 16u) | (packed.b << 8u) | packed.a;
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
        SRGB_TO_LINEAR[color.r],
        SRGB_TO_LINEAR[color.g],
        SRGB_TO_LINEAR[color.b],
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
