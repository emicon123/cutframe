// Output of the canvas (docs/design/render-graph.md §4.2 step 4, §8): a bilinear scale of the
// Rgba16Float canvas into an 8-bit target, black outside the canvas rectangle (letterbox). The
// preview presents with it; the offline output pass uses it 1:1 into Rgba8Unorm. R'G'B' are
// written as they are (the target is never an sRGB format); the premultiplied canvas is shown
// over black, so the output is opaque.

struct Params {
    // The canvas rectangle in target pixels: xy origin, zw size.
    rect: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var canvas_sampler: sampler;
@group(0) @binding(2) var canvas: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the target.
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (position.xy - params.rect.xy) / params.rect.zw;
    // Sampled before the branch (uniform control flow).
    let c = textureSample(canvas, canvas_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
