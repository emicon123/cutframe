// Stage-0 test pattern: proves the GPU path end to end.
// Draws the project canvas letterboxed into the viewport, with an animated gradient,
// a moving bar, a centre cross and a title-safe frame.

struct Uniforms {
    time: f32,
    viewport_aspect: f32,
    canvas_aspect: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Full-screen triangle, no vertex buffer.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var out: VsOut;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn hsv_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
    let p = abs(fract(c.xxx + k) * 6.0 - 3.0);
    return c.z * mix(vec3<f32>(1.0), clamp(p - 1.0, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Letterbox: map viewport uv to canvas uv, keeping the canvas aspect ratio.
    var scale = vec2<f32>(1.0, 1.0);
    if (u.viewport_aspect > u.canvas_aspect) {
        scale.x = u.viewport_aspect / u.canvas_aspect;
    } else {
        scale.y = u.canvas_aspect / u.viewport_aspect;
    }
    let uv = (in.uv - 0.5) * scale + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.035, 0.035, 0.045, 1.0);
    }

    var color = hsv_to_rgb(vec3<f32>(fract(uv.x * 0.5 + u.time * 0.05), 0.55, 0.35 + 0.25 * uv.y));

    // Moving vertical bar (one sweep every 4 s).
    let bar = fract(u.time * 0.25);
    if (abs(uv.x - bar) < 0.004) {
        color = vec3<f32>(1.0);
    }

    // Centre cross.
    let centre = abs(uv - 0.5);
    if ((centre.x < 0.0015 && centre.y < 0.03) || (centre.y < 0.0015 * u.canvas_aspect && centre.x < 0.03 / u.canvas_aspect)) {
        color = vec3<f32>(1.0);
    }

    // Title-safe frame at 90 %.
    let safe = abs(uv - 0.5) * 2.0;
    let edge = max(safe.x, safe.y);
    if (edge > 0.896 && edge < 0.9 && all(safe <= vec2<f32>(0.9))) {
        color = mix(color, vec3<f32>(1.0), 0.6);
    }

    return vec4<f32>(color, 1.0);
}
