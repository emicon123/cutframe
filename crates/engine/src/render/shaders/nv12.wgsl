// Export NV12 pass (docs/design/export.md §4.3): the Rgba16Float canvas (premultiplied R'G'B'A,
// shown over black, so R'G'B' are used as they are) to BT.709 limited-range Y' and CbCr planes.
//
// - fs_luma writes one R8Unorm texel per canvas pixel: code 16 + 219 Y'.
// - fs_chroma writes one Rg8Unorm texel per 2 x 2 block, left chroma siting (MPEG-2 / H.264
//   default): horizontal weights 1/4, 1/2, 1/4 on luma columns 2i - 1, 2i, 2i + 1, vertical
//   1/2, 1/2 on rows 2j, 2j + 1 (clamped at the edges); code 128 + 224 C.
//
// `textureLoad` only, f32, every code value rounded here (floor(x + 0.5)) and written as
// code / 255, so the unorm store is exact. Mirrors `engine::export::nv12::reference_nv12`
// operation by operation; the WARP parity test (`tests/export_nv12.rs`) holds them together.

struct Params {
    // Rows of the R'G'B' to Y'CbCr matrix (engine::color::YuvMatrix::Bt709.from_rgb()).
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var canvas: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the target.
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}

fn rgb_at(x: i32, y: i32) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(canvas));
    let p = clamp(vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
    return clamp(textureLoad(canvas, p, 0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn code(value: f32) -> f32 {
    return floor(value + 0.5) / 255.0;
}

@fragment
fn fs_luma(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    let rgb = rgb_at(p.x, p.y);
    let y = dot(params.m0.xyz, rgb);
    return vec4<f32>(code(16.0 + 219.0 * y), 0.0, 0.0, 1.0);
}

fn row_filtered(x: i32, y: i32) -> vec3<f32> {
    return 0.25 * rgb_at(x - 1, y) + 0.5 * rgb_at(x, y) + 0.25 * rgb_at(x + 1, y);
}

@fragment
fn fs_chroma(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    let x = 2 * p.x;
    let y = 2 * p.y;
    let rgb = 0.5 * row_filtered(x, y) + 0.5 * row_filtered(x, y + 1);
    let cb = dot(params.m1.xyz, rgb);
    let cr = dot(params.m2.xyz, rgb);
    return vec4<f32>(code(128.0 + 224.0 * cb), code(128.0 + 224.0 * cr), 0.0, 1.0);
}
