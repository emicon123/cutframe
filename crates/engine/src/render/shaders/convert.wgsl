// Source convert (docs/design/render-graph.md §5.2): one decoded frame to an upright Rgba16Float
// texture of working R'G'B'A (Rec.709 primaries, BT.1886 encoding, premultiplied alpha).
//
// Mirrors `engine::color::convert_pixel` operation by operation; the WARP parity test
// (`render/convert_parity.rs`) holds the two together. Change both or neither.

struct Params {
    // xy: UV scale of plane 0 (visible / texture size), zw: of the chroma planes.
    plane_scale: vec4<f32>,
    // xy: centre of the last texel of plane 0 that holds picture, zw: of the chroma planes. UVs
    // are clamped to it, so bilinear taps never read coded padding.
    plane_max: vec4<f32>,
    // xy: 1 / output size.
    out_texel: vec4<f32>,
    // x: sample to code value, y: luma offset, z: luma scale, w: chroma offset.
    code: vec4<f32>,
    // x: chroma scale.
    range: vec4<f32>,
    // Rows of the Y'CbCr to R'G'B' matrix (identity for RGB sources).
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // x: PQinv(Lsrc), y: maxLum, z: knee start KS (BT.2390 EETF).
    tone: vec4<f32>,
    // x: layout (0 planar, 1 semi-planar, 2 packed), y: transfer (0 SDR, 1 SDR BT.2020, 2 PQ,
    // 3 HLG), z: clockwise quarter turns, w: 1 = straight alpha to premultiply.
    modes: vec4<u32>,
    // xy: taps per output pixel (k x k downscale filter).
    taps: vec4<u32>,
};

@group(0) @binding(0) var<uniform> params: Params;
// Y' (or packed RGBA); planar: Cb, semi-planar: CbCr; planar: Cr.
@group(0) @binding(2) var plane0: texture_2d<f32>;
@group(0) @binding(3) var plane1: texture_2d<f32>;
@group(0) @binding(4) var plane2: texture_2d<f32>;

const SDR_WHITE_NITS: f32 = 203.0;
const HLG_DISPLAY_PEAK_NITS: f32 = 1000.0;
const HLG_SYSTEM_GAMMA: f32 = 1.2;
const PQ_MAX_NITS: f32 = 10000.0;
const DISPLAY_GAMMA: f32 = 2.4;
const LUMA_BT2020: vec3<f32> = vec3<f32>(0.2627, 0.6780, 0.0593);

const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;

const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 0.28466892;
const HLG_C: f32 = 0.5599107;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the target.
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}

fn pow0(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    return pow(x, y);
}

fn bt2020_to_bt709(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        1.660491 * c.x + -0.587641 * c.y + -0.072850 * c.z,
        -0.124550 * c.x + 1.132900 * c.y + -0.008349 * c.z,
        -0.018151 * c.x + -0.100579 * c.y + 1.118730 * c.z,
    );
}

fn bt1886_encode(c: vec3<f32>) -> vec3<f32> {
    let inv = 1.0 / DISPLAY_GAMMA;
    return vec3<f32>(pow0(c.x, inv), pow0(c.y, inv), pow0(c.z, inv));
}

fn pq_eotf(signal: f32) -> f32 {
    let p = pow0(max(signal, 0.0), 1.0 / PQ_M2);
    let num = max(p - PQ_C1, 0.0);
    let den = PQ_C2 - PQ_C3 * p;
    return pow0(num / den, 1.0 / PQ_M1) * PQ_MAX_NITS;
}

fn pq_inverse_eotf(nits: f32) -> f32 {
    let y = pow0(clamp(nits / PQ_MAX_NITS, 0.0, 1.0), PQ_M1);
    return pow0((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y), PQ_M2);
}

fn hlg_inverse_oetf(signal: f32) -> f32 {
    let s = max(signal, 0.0);
    if s <= 0.5 {
        return s * s / 3.0;
    }
    return (exp((s - HLG_C) / HLG_A) + HLG_B) / 12.0;
}

// BT.2390 EETF on display light in nits, identity below the knee.
fn eetf_nits(nits: f32) -> f32 {
    let source_pq = params.tone.x;
    let max_lum = params.tone.y;
    let ks = params.tone.z;
    let e1 = pq_inverse_eotf(nits) / source_pq;
    if e1 <= ks {
        return nits;
    }
    let t = (min(e1, 1.0) - ks) / (1.0 - ks);
    let t2 = t * t;
    let t3 = t2 * t;
    let e2 = (2.0 * t3 - 3.0 * t2 + 1.0) * ks
        + (t3 - 2.0 * t2 + t) * (1.0 - ks)
        + (-2.0 * t3 + 3.0 * t2) * max_lum;
    return pq_eotf(e2 * source_pq);
}

fn tone_map(signal_in: vec3<f32>, hlg: bool) -> vec3<f32> {
    let signal = clamp(signal_in, vec3<f32>(0.0), vec3<f32>(1.0));
    var nits: vec3<f32>;
    if hlg {
        let scene = vec3<f32>(
            hlg_inverse_oetf(signal.x),
            hlg_inverse_oetf(signal.y),
            hlg_inverse_oetf(signal.z),
        );
        let ys = LUMA_BT2020.x * scene.x + LUMA_BT2020.y * scene.y + LUMA_BT2020.z * scene.z;
        nits = scene * (HLG_DISPLAY_PEAK_NITS * pow0(ys, HLG_SYSTEM_GAMMA - 1.0));
    } else {
        nits = vec3<f32>(pq_eotf(signal.x), pq_eotf(signal.y), pq_eotf(signal.z));
    }
    let m = max(max(nits.x, nits.y), nits.z);
    var toned = vec3<f32>(0.0);
    if m > 0.0 {
        toned = nits * (eetf_nits(m) / m);
    }
    let linear = bt2020_to_bt709(toned / SDR_WHITE_NITS);
    return bt1886_encode(clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0)));
}

// Code values to working R'G'B' (`engine::color::convert_pixel`).
fn convert(code: vec3<f32>) -> vec3<f32> {
    let e = vec3<f32>(
        (code.x - params.code.y) * params.code.z,
        (code.y - params.code.w) * params.range.x,
        (code.z - params.code.w) * params.range.x,
    );
    let rgb = vec3<f32>(
        params.m0.x * e.x + params.m0.y * e.y + params.m0.z * e.z,
        params.m1.x * e.x + params.m1.y * e.y + params.m1.z * e.z,
        params.m2.x * e.x + params.m2.y * e.y + params.m2.z * e.z,
    );
    switch params.modes.y {
        case 1u: {
            let lin = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
            let linear = vec3<f32>(
                pow0(lin.x, DISPLAY_GAMMA),
                pow0(lin.y, DISPLAY_GAMMA),
                pow0(lin.z, DISPLAY_GAMMA),
            );
            return bt1886_encode(clamp(bt2020_to_bt709(linear), vec3<f32>(0.0), vec3<f32>(1.0)));
        }
        case 2u: {
            return tone_map(rgb, false);
        }
        case 3u: {
            return tone_map(rgb, true);
        }
        default: {
            return clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }
}

// Upright output coordinates to stored-picture coordinates (clockwise quarter turns).
fn source_uv(p: vec2<f32>) -> vec2<f32> {
    switch params.modes.z {
        case 1u: {
            return vec2<f32>(p.y, 1.0 - p.x);
        }
        case 2u: {
            return vec2<f32>(1.0 - p.x, 1.0 - p.y);
        }
        case 3u: {
            return vec2<f32>(1.0 - p.y, p.x);
        }
        default: {
            return p;
        }
    }
}

// Bilinear filtering in f32 at `uv` (clamped to the texture's edge), done here instead of by
// the sampler: GPUs filter with a few bits of weight precision, which depends on the plane
// layout (P010 and LSB-aligned 10-bit planes differ in the last bits) and which the HDR gamut
// conversion amplifies near the gamut boundary. This way every GPU and every layout of the same
// frame give the same values.
fn bilinear(plane: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(plane));
    let t = max(uv * vec2<f32>(size) - 0.5, vec2<f32>(0.0));
    let last = size - vec2<i32>(1);
    let i0 = min(vec2<i32>(floor(t)), last);
    let i1 = min(i0 + vec2<i32>(1), last);
    let f = t - floor(t);
    let a = textureLoad(plane, i0, 0);
    let b = textureLoad(plane, vec2<i32>(i1.x, i0.y), 0);
    let c = textureLoad(plane, vec2<i32>(i0.x, i1.y), 0);
    let d = textureLoad(plane, i1, 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Raw samples (unorm) at `uv` of the visible picture: Y', Cb, Cr, 1 or R, G, B, A.
fn fetch(uv: vec2<f32>) -> vec4<f32> {
    let luma_uv = min(uv * params.plane_scale.xy, params.plane_max.xy);
    let chroma_uv = min(uv * params.plane_scale.zw, params.plane_max.zw);
    switch params.modes.x {
        case 0u: {
            return vec4<f32>(
                bilinear(plane0, luma_uv).r,
                bilinear(plane1, chroma_uv).r,
                bilinear(plane2, chroma_uv).r,
                1.0,
            );
        }
        case 1u: {
            let cbcr = bilinear(plane1, chroma_uv).rg;
            return vec4<f32>(bilinear(plane0, luma_uv).r, cbcr, 1.0);
        }
        default: {
            return bilinear(plane0, luma_uv);
        }
    }
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = position.xy * params.out_texel.xy;
    let k = vec2<f32>(f32(params.taps.x), f32(params.taps.y));
    let straight = params.modes.w == 1u;
    var sum = vec4<f32>(0.0);
    for (var j = 0u; j < params.taps.y; j++) {
        for (var i = 0u; i < params.taps.x; i++) {
            let offset = ((vec2<f32>(f32(i), f32(j)) + 0.5) / k - 0.5) * params.out_texel.xy;
            var s = fetch(source_uv(p + offset));
            if straight {
                s = vec4<f32>(s.rgb * s.a, s.a);
            }
            sum += s;
        }
    }
    var s = sum / (k.x * k.y);
    var alpha = 1.0;
    if straight {
        alpha = s.a;
        if alpha <= 0.0 {
            return vec4<f32>(0.0);
        }
        s = vec4<f32>(s.rgb / alpha, alpha);
    }
    let rgb = convert(s.rgb * params.code.x);
    return vec4<f32>(rgb * alpha, alpha);
}
