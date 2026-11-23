// Layer pass (docs/design/render-graph.md §4.2 step 3, §4.3): one quad per layer, blended
// premultiplied-over (src + dst x (1 - src.a)) into the Rgba16Float canvas.
//
// Modes: 0 = a converted source texture (premultiplied R'G'B'A), 1 = a solid colour, 2 = the
// placeholder's diagonal stripes in two colours. Opacity scales the premultiplied result.

struct Layer {
    // The unit quad (u, v in 0..=1, v down) to render pixels: x = dot(row0.xyz, (u, v, 1)).
    row0: vec4<f32>,
    row1: vec4<f32>,
    // xy: render size in pixels, zw: render pixels per canvas pixel.
    dest: vec4<f32>,
    // xy: source UV at the quad's top-left corner, zw: at its bottom-right corner (crop).
    uv: vec4<f32>,
    // Premultiplied colour (solid) or the first stripe colour.
    color: vec4<f32>,
    // The second stripe colour (premultiplied).
    color2: vec4<f32>,
    // x: opacity, y: stripe period in canvas pixels (a whole number).
    params: vec4<f32>,
    // x: mode.
    mode: vec4<u32>,
};

@group(0) @binding(0) var<uniform> layer: Layer;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // Two triangles: (0,0) (1,0) (0,1) and (0,1) (1,0) (1,1).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let unit = corners[index];
    let p = vec3<f32>(unit, 1.0);
    let pixel = vec2<f32>(dot(layer.row0.xyz, p), dot(layer.row1.xyz, p));
    let ndc = vec2<f32>(pixel.x / layer.dest.x * 2.0 - 1.0, 1.0 - pixel.y / layer.dest.y * 2.0);
    var out: VertexOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(layer.uv.xy, layer.uv.zw, unit);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let opacity = layer.params.x;
    // Sampled in every mode (uniform control flow); solid modes bind a 1x1 texture.
    let texel = textureSample(source, source_sampler, in.uv);
    var color: vec4<f32>;
    switch layer.mode.x {
        case 0u: {
            color = texel;
        }
        case 1u: {
            color = layer.color;
        }
        default: {
            // Diagonal stripes, fixed in canvas pixels so every render size shows the same
            // pattern. The period is a whole number of canvas pixels and the bands are offset by
            // half a pixel, so no pixel centre lies on a band edge (where rounding would differ
            // between GPUs).
            let canvas = in.position.xy / layer.dest.zw;
            let band = floor((canvas.x + canvas.y + 0.5) / layer.params.y);
            if (i32(band) & 1) == 0 {
                color = layer.color;
            } else {
                color = layer.color2;
            }
        }
    }
    return color * opacity;
}
