//! Colour reference functions: the transfer curves, the HDR tone map, the YCbCr matrices and range
//! expansion of the source-convert pass (design `docs/design/render-graph.md` §5.2), as plain `f32`
//! Rust.
//!
//! These are the definition the engine's colour code is held to:
//!
//! - the source-convert shader (`render/shaders/convert.wgsl`) mirrors them operation by
//!   operation and is tested against [`convert_pixel`] on the software adapter (§12);
//! - HDR posters and filmstrips (`media::tonemap`) call [`HdrToneMap::map`] directly, so the media
//!   panel and the preview use the same operator.
//!
//! Working values are display-referred R'G'B' with Rec.709 primaries and BT.1886 encoding
//! (`V = L^(1/2.4)`), SDR range `0..=1` (§5.1).

use app_core::media::{HdrInfo, HdrTransfer};

use crate::render::{ConvertParams, TransferPath};

/// SDR reference white in nits (BT.2408). HDR sources are tone-mapped so that their peak lands
/// here.
pub const SDR_WHITE_NITS: f32 = 203.0;
/// Nominal peak of the display the HLG OOTF is evaluated for (BT.2100).
pub const HLG_DISPLAY_PEAK_NITS: f32 = 1000.0;
/// HLG system gamma for a 1000-nit display (BT.2100).
pub const HLG_SYSTEM_GAMMA: f32 = 1.2;
/// The PQ signal range in nits (ST 2084).
pub const PQ_MAX_NITS: f32 = 10_000.0;
/// PQ source peak when the stream carries neither MaxCLL nor a mastering display peak.
pub const DEFAULT_PQ_PEAK_NITS: u32 = 1000;
/// The range a PQ source peak is clamped to.
pub const PQ_PEAK_RANGE: std::ops::RangeInclusive<u32> = 400..=10_000;
/// Gamma of BT.1886 with a zero black level, also used to linearise SDR BT.2020 sources.
pub const DISPLAY_GAMMA: f32 = 2.4;

/// BT.2100 luminance weights of BT.2020 primaries (`Ys` of the HLG OOTF).
pub const LUMA_BT2020: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// Linear BT.2020 RGB to linear BT.709 RGB (BT.2087).
pub const BT2020_TO_BT709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641, -0.072_850],
    [-0.124_550, 1.132_9, -0.008_349],
    [-0.018_151, -0.100_579, 1.118_73],
];

/// The 3x3 identity, the matrix of RGB sources.
pub const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

// SMPTE ST 2084 constants.
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;

// BT.2100 HLG constants.
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_7;

/// `x^y` for `x >= 0`, with `0^y = 0` (`y > 0`). The shader uses the same guard, because WGSL
/// leaves `pow(0, y)` to the implementation.
fn pow0(x: f32, y: f32) -> f32 {
    if x <= 0.0 { 0.0 } else { x.powf(y) }
}

/// PQ EOTF (SMPTE ST 2084): non-linear signal `0..=1` to display light in nits.
pub fn pq_eotf(signal: f32) -> f32 {
    let p = pow0(signal.max(0.0), 1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * p;
    pow0(num / den, 1.0 / PQ_M1) * PQ_MAX_NITS
}

/// Inverse PQ EOTF: display light in nits to the non-linear signal `0..=1`.
pub fn pq_inverse_eotf(nits: f32) -> f32 {
    let y = pow0((nits / PQ_MAX_NITS).clamp(0.0, 1.0), PQ_M1);
    pow0((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y), PQ_M2)
}

/// HLG OETF (BT.2100): scene light `0..=1` to the non-linear signal `0..=1`.
pub fn hlg_oetf(scene: f32) -> f32 {
    let e = scene.max(0.0);
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt()
    } else {
        HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
    }
}

/// HLG inverse OETF (BT.2100): non-linear signal `0..=1` to scene light `0..=1`.
pub fn hlg_inverse_oetf(signal: f32) -> f32 {
    let s = signal.max(0.0);
    if s <= 0.5 {
        s * s / 3.0
    } else {
        (((s - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// HLG OOTF for a [`HLG_DISPLAY_PEAK_NITS`] display: scene light (BT.2020 RGB, `0..=1`) to
/// display light in nits, `Fd = Lw * Ys^(gamma - 1) * E`.
pub fn hlg_ootf(scene: [f32; 3]) -> [f32; 3] {
    let ys = dot(LUMA_BT2020, scene);
    let gain = HLG_DISPLAY_PEAK_NITS * pow0(ys, HLG_SYSTEM_GAMMA - 1.0);
    scene.map(|e| gain * e)
}

/// The peak luminance `Lsrc` the tone map compresses to SDR white, in nits: PQ uses MaxCLL, else
/// the mastering display peak, else [`DEFAULT_PQ_PEAK_NITS`], clamped to [`PQ_PEAK_RANGE`]; HLG
/// uses [`HLG_DISPLAY_PEAK_NITS`]. A zero value counts as missing (some encoders write
/// `MaxCLL = 0` for "unknown").
pub fn source_peak_nits(hdr: &HdrInfo) -> u32 {
    match hdr.transfer {
        HdrTransfer::Hlg => HLG_DISPLAY_PEAK_NITS as u32,
        HdrTransfer::Pq | HdrTransfer::DolbyVisionOnly => hdr
            .max_cll_nits
            .filter(|&n| n > 0)
            .or(hdr.mastering_max_nits.filter(|&n| n > 0))
            .unwrap_or(DEFAULT_PQ_PEAK_NITS)
            .clamp(*PQ_PEAK_RANGE.start(), *PQ_PEAK_RANGE.end()),
    }
}

/// The BT.2390 EETF from `[0, Lsrc]` to `[0, SDR_WHITE_NITS]`, evaluated in the PQ domain
/// normalised to the source peak (source black is 0, so there is no black-level lift).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eetf {
    /// `PQinv(Lsrc)`: the normalisation of the PQ domain.
    pub source_pq: f32,
    /// `PQinv(203) / PQinv(Lsrc)`: the normalised target peak.
    pub max_lum: f32,
    /// Knee start `KS = 1.5 * maxLum - 0.5`; the curve is the identity below it.
    pub knee: f32,
}

impl Eetf {
    pub fn new(source_peak_nits: f32) -> Self {
        let source_pq = pq_inverse_eotf(source_peak_nits);
        let max_lum = pq_inverse_eotf(SDR_WHITE_NITS) / source_pq;
        Self {
            source_pq,
            max_lum,
            knee: 1.5 * max_lum - 0.5,
        }
    }

    /// The curve on the normalised PQ signal `e1` (`0..=1`, values above 1 are clipped).
    pub fn map_normalized(&self, e1: f32) -> f32 {
        if e1 <= self.knee {
            return e1;
        }
        let ks = self.knee;
        let t = (e1.min(1.0) - ks) / (1.0 - ks);
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * ks
            + (t3 - 2.0 * t2 + t) * (1.0 - ks)
            + (-2.0 * t3 + 3.0 * t2) * self.max_lum
    }

    /// The curve on display light in nits. Exactly the identity below the knee.
    pub fn map_nits(&self, nits: f32) -> f32 {
        let e1 = pq_inverse_eotf(nits) / self.source_pq;
        if e1 <= self.knee {
            return nits;
        }
        pq_eotf(self.map_normalized(e1) * self.source_pq)
    }
}

/// The HDR curve of a source, as the tone map sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HdrCurve {
    Pq,
    Hlg,
}

/// HDR to SDR for one source: transfer curve to display light, BT.2390 EETF on `max(R, G, B)`,
/// BT.2020 to BT.709, clip, BT.1886 encode (§5.2 step 5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdrToneMap {
    pub curve: HdrCurve,
    pub eetf: Eetf,
}

impl HdrToneMap {
    /// The tone map for `curve` with source peak `peak_nits` (see [`source_peak_nits`]).
    pub fn new(curve: HdrCurve, peak_nits: u32) -> Self {
        Self {
            curve,
            eetf: Eetf::new(peak_nits as f32),
        }
    }

    /// The tone map of an HDR stream. `DolbyVisionOnly` takes the PQ path.
    pub fn for_hdr(hdr: &HdrInfo) -> Self {
        let curve = match hdr.transfer {
            HdrTransfer::Hlg => HdrCurve::Hlg,
            HdrTransfer::Pq | HdrTransfer::DolbyVisionOnly => HdrCurve::Pq,
        };
        Self::new(curve, source_peak_nits(hdr))
    }

    /// Non-linear BT.2020 R'G'B' (`0..=1`, clipped) to working R'G'B' (BT.709, BT.1886, `0..=1`).
    pub fn map(&self, signal: [f32; 3]) -> [f32; 3] {
        let signal = signal.map(|v| v.clamp(0.0, 1.0));
        let nits = match self.curve {
            HdrCurve::Pq => signal.map(pq_eotf),
            HdrCurve::Hlg => hlg_ootf(signal.map(hlg_inverse_oetf)),
        };
        let m = nits[0].max(nits[1]).max(nits[2]);
        let toned = if m > 0.0 {
            let gain = self.eetf.map_nits(m) / m;
            nits.map(|v| v * gain)
        } else {
            [0.0; 3]
        };
        let relative = toned.map(|v| v / SDR_WHITE_NITS);
        mat3_mul(&BT2020_TO_BT709, relative).map(|v| bt1886_encode(v.clamp(0.0, 1.0)))
    }
}

/// Inverse BT.1886 with a zero black level: relative display light `0..=1` to the signal.
pub fn bt1886_encode(light: f32) -> f32 {
    pow0(light, 1.0 / DISPLAY_GAMMA)
}

/// BT.1886 with a zero black level: the signal to relative display light.
pub fn bt1886_decode(signal: f32) -> f32 {
    pow0(signal, DISPLAY_GAMMA)
}

/// SDR R'G'B' with BT.2020 primaries to working R'G'B': linearise with gamma 2.4, convert to
/// BT.709, clip, re-encode (§5.2 step 4).
pub fn sdr_bt2020_to_bt709(signal: [f32; 3]) -> [f32; 3] {
    let linear = signal.map(|v| bt1886_decode(v.clamp(0.0, 1.0)));
    mat3_mul(&BT2020_TO_BT709, linear).map(|v| bt1886_encode(v.clamp(0.0, 1.0)))
}

/// A Y'CbCr to R'G'B' matrix (non-constant luminance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
    Bt2020Ncl,
}

impl YuvMatrix {
    /// The matrix of FFmpeg colour-space name `tag` (`av_color_space_name`). Untagged and unknown
    /// sources use BT.709 when the coded height is 720 or more, else BT.601 (FFmpeg's and most
    /// players' convention). SMPTE 240M is close enough to BT.709 and FCC to BT.601; BT.2020
    /// constant luminance is treated as non-constant (rare, the difference is small).
    pub fn from_tag(tag: Option<&str>, coded_height: u32) -> Self {
        match tag {
            Some("bt709" | "smpte240m") => Self::Bt709,
            Some("bt470bg" | "smpte170m" | "fcc") => Self::Bt601,
            Some("bt2020nc" | "bt2020c") => Self::Bt2020Ncl,
            _ if coded_height >= 720 => Self::Bt709,
            _ => Self::Bt601,
        }
    }

    /// `(Kr, Kb)`.
    pub fn coefficients(self) -> (f32, f32) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
            Self::Bt2020Ncl => (0.2627, 0.0593),
        }
    }

    /// Rows that map `(Y', Cb, Cr)` (Y' in `0..=1`, Cb and Cr in `-0.5..=0.5`) to R'G'B'.
    pub fn to_rgb(self) -> [[f32; 3]; 3] {
        let (kr, kb) = self.coefficients();
        let kg = 1.0 - kr - kb;
        let cr_r = 2.0 * (1.0 - kr);
        let cb_b = 2.0 * (1.0 - kb);
        [
            [1.0, 0.0, cr_r],
            [1.0, -kb * cb_b / kg, -kr * cr_r / kg],
            [1.0, cb_b, 0.0],
        ]
    }

    /// The inverse of [`Self::to_rgb`]: R'G'B' to `(Y', Cb, Cr)`.
    pub fn from_rgb(self) -> [[f32; 3]; 3] {
        let (kr, kb) = self.coefficients();
        let kg = 1.0 - kr - kb;
        [
            [kr, kg, kb],
            [-kr / (2.0 * (1.0 - kb)), -kg / (2.0 * (1.0 - kb)), 0.5],
            [0.5, -kg / (2.0 * (1.0 - kr)), -kb / (2.0 * (1.0 - kr))],
        ]
    }
}

/// Quantisation range of code values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRange {
    /// "TV" range: Y' in `16..=235`, Cb/Cr in `16..=240` at 8 bits.
    Limited,
    /// "PC" range: the full code range.
    Full,
}

impl ColorRange {
    /// The range of FFmpeg range name `tag` (`tv`, `pc`). Unknown: full for RGB, grey and the
    /// deprecated full-range `yuvj*` formats (`pixel_format` is FFmpeg's name), limited for every
    /// other YUV format.
    pub fn from_tag(tag: Option<&str>, pixel_format: &str, rgb: bool) -> Self {
        match tag {
            Some("pc") => Self::Full,
            Some("tv") => Self::Limited,
            _ if rgb || pixel_format.starts_with("yuvj") || pixel_format.starts_with("gray") => {
                Self::Full
            }
            _ => Self::Limited,
        }
    }
}

/// Range expansion of one source: `value = (code - offset) * scale` per channel. Luma (and every
/// RGB channel) maps to `0..=1`, chroma to `-0.5..=0.5`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeCoefficients {
    pub y_offset: f32,
    pub y_scale: f32,
    pub c_offset: f32,
    pub c_scale: f32,
}

impl RangeCoefficients {
    /// The coefficients for `bits`-bit code values (§5.2 step 2). Limited:
    /// `Y' = (code - 16 * 2^(b-8)) / (219 * 2^(b-8))`, `C = (code - 128 * 2^(b-8)) /
    /// (224 * 2^(b-8))`. Full: `Y' = code / (2^b - 1)`, `C = (code - 2^(b-1)) / (2^b - 1)`. RGB
    /// sources use the luma rule for all three channels.
    pub fn new(range: ColorRange, bits: u8, rgb: bool) -> Self {
        let bits = bits.clamp(8, 16);
        let step = f32::from(1u16 << (bits - 8));
        let max = ((1u32 << bits) - 1) as f32;
        let (y_offset, y_scale, c_offset, c_scale) = match range {
            ColorRange::Limited => (
                16.0 * step,
                1.0 / (219.0 * step),
                128.0 * step,
                1.0 / (224.0 * step),
            ),
            ColorRange::Full => (0.0, 1.0 / max, f32::from(1u16 << (bits - 1)), 1.0 / max),
        };
        if rgb {
            Self {
                y_offset,
                y_scale,
                c_offset: y_offset,
                c_scale: y_scale,
            }
        } else {
            Self {
                y_offset,
                y_scale,
                c_offset,
                c_scale,
            }
        }
    }

    pub fn expand(&self, code: [f32; 3]) -> [f32; 3] {
        [
            (code[0] - self.y_offset) * self.y_scale,
            (code[1] - self.c_offset) * self.c_scale,
            (code[2] - self.c_offset) * self.c_scale,
        ]
    }
}

/// The matrix of a source: its Y'CbCr matrix, or the identity for RGB sources (`None`).
pub fn source_matrix(matrix: Option<YuvMatrix>) -> [[f32; 3]; 3] {
    matrix.map_or(IDENTITY, YuvMatrix::to_rgb)
}

/// The whole source convert of one pixel, the exact reference of `convert.wgsl`: `code` holds the
/// `bits`-bit code values (Y', Cb, Cr, or R, G, B for RGB sources; fractional after filtering),
/// the result is working R'G'B' in `0..=1`. Alpha is not part of it: the pass premultiplies the
/// result by the source alpha for images.
pub fn convert_pixel(code: [f32; 3], bits: u8, params: &ConvertParams) -> [f32; 3] {
    let range = RangeCoefficients::new(params.range, bits, params.matrix.is_none());
    let rgb = mat3_mul(&source_matrix(params.matrix), range.expand(code));
    match params.transfer {
        TransferPath::Sdr => rgb.map(|v| v.clamp(0.0, 1.0)),
        TransferPath::SdrBt2020 => sdr_bt2020_to_bt709(rgb),
        TransferPath::Pq { peak_nits } => HdrToneMap::new(HdrCurve::Pq, peak_nits).map(rgb),
        TransferPath::Hlg => HdrToneMap::new(HdrCurve::Hlg, HLG_DISPLAY_PEAK_NITS as u32).map(rgb),
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `m * v` for a row-major 3x3 matrix.
pub fn mat3_mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

#[cfg(test)]
mod tests {
    use app_core::media::Rotation;

    use super::*;
    use crate::render::AlphaMode;

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[track_caller]
    fn assert_close3(got: [f32; 3], expected: [f32; 3]) {
        assert!(
            (0..3).all(|c| close(got[c], expected[c], 1e-6)),
            "{got:?} != {expected:?}"
        );
    }

    #[test]
    fn pq_reference_points() {
        // BT.2408: 203 nits is PQ 0.58 (58 %).
        assert!(close(pq_inverse_eotf(203.0), 0.5806, 1e-4));
        assert!(close(pq_inverse_eotf(10_000.0), 1.0, 1e-6));
        assert!(close(pq_inverse_eotf(100.0), 0.5081, 1e-4));
        assert!(close(pq_inverse_eotf(1000.0), 0.7518, 1e-4));
        assert_eq!(pq_eotf(0.0), 0.0);
        assert!(close(pq_eotf(1.0), 10_000.0, 0.5));
        for nits in [0.01, 1.0, 50.0, 203.0, 1000.0, 4000.0] {
            let back = pq_eotf(pq_inverse_eotf(nits));
            assert!(close(back, nits, nits * 1e-4), "{nits} -> {back}");
        }
    }

    #[test]
    fn hlg_reference_points() {
        assert!(close(hlg_oetf(1.0 / 12.0), 0.5, 1e-6));
        assert!(close(hlg_oetf(1.0), 1.0, 1e-6));
        assert_eq!(hlg_oetf(0.0), 0.0);
        assert!(close(hlg_inverse_oetf(0.5), 1.0 / 12.0, 1e-7));
        for e in [0.0, 0.01, 1.0 / 12.0, 0.3, 0.9, 1.0] {
            assert!(close(hlg_inverse_oetf(hlg_oetf(e)), e, 1e-6), "{e}");
        }
        // The OOTF maps scene white to the display peak and HLG 75 % grey to about 203 nits.
        assert!(close(hlg_ootf([1.0; 3])[0], 1000.0, 1e-2));
        let grey = hlg_ootf([hlg_inverse_oetf(0.75); 3])[0];
        assert!(close(grey, 203.0, 2.0), "{grey}");
        assert_eq!(hlg_ootf([0.0; 3]), [0.0; 3]);
    }

    #[test]
    fn eetf_is_identity_below_the_knee_monotonic_and_maps_the_peak_to_white() {
        for peak in [400.0, 1000.0, 4000.0, 10_000.0] {
            let eetf = Eetf::new(peak);
            assert!(
                eetf.knee > 0.0 && eetf.knee < eetf.max_lum,
                "{peak}: {eetf:?}"
            );
            let knee_nits = pq_eotf(eetf.knee * eetf.source_pq);
            for nits in [0.0, 1.0, 10.0, knee_nits * 0.99] {
                assert_eq!(eetf.map_nits(nits), nits, "{peak}: identity at {nits}");
            }
            let top = eetf.map_nits(peak);
            assert!(close(top, SDR_WHITE_NITS, 0.05), "{peak}: peak -> {top}");
            assert!(
                close(eetf.map_nits(peak * 4.0), top, 1e-3),
                "clipped above Lsrc"
            );
            // KS = 1.5 maxLum - 0.5 makes the slope vanish at the top (T = 1), where f32 rounding
            // may wobble by an ulp: monotonic within 1e-6.
            let mut last = 0.0;
            for i in 0..=2000 {
                let e = eetf.map_normalized(i as f32 / 2000.0);
                assert!(e >= last - 1e-6, "{peak}: not monotonic at {i}");
                assert!(e <= eetf.max_lum + 1e-6);
                last = last.max(e);
            }
        }
    }

    #[test]
    fn source_peak_rule() {
        let hdr = |transfer, cll, mastering| HdrInfo {
            transfer,
            mastering_max_nits: mastering,
            max_cll_nits: cll,
            dolby_vision: None,
        };
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::Pq, Some(1500), Some(4000))),
            1500
        );
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::Pq, None, Some(4000))),
            4000
        );
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::Pq, Some(0), Some(4000))),
            4000
        );
        assert_eq!(source_peak_nits(&hdr(HdrTransfer::Pq, None, None)), 1000);
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::Pq, Some(100), None)),
            400
        );
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::Pq, None, Some(20_000))),
            10_000
        );
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::DolbyVisionOnly, Some(600), None)),
            600
        );
        assert_eq!(
            source_peak_nits(&hdr(HdrTransfer::Hlg, Some(4000), None)),
            1000
        );
    }

    #[test]
    fn matrices_round_trip() {
        for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709, YuvMatrix::Bt2020Ncl] {
            let to = matrix.to_rgb();
            let from = matrix.from_rgb();
            for rgb in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.2, 0.5, 0.9], [1.0; 3]] {
                let back = mat3_mul(&to, mat3_mul(&from, rgb));
                for c in 0..3 {
                    assert!(
                        close(back[c], rgb[c], 1e-5),
                        "{matrix:?} {rgb:?} -> {back:?}"
                    );
                }
            }
            // White has no chroma, and the luma weights sum to 1.
            let white = mat3_mul(&from, [1.0; 3]);
            assert!(close(white[0], 1.0, 1e-6) && close(white[1], 0.0, 1e-6));
            assert!(close(white[2], 0.0, 1e-6));
        }
        assert_eq!(YuvMatrix::from_tag(None, 1080), YuvMatrix::Bt709);
        assert_eq!(YuvMatrix::from_tag(Some("unknown"), 720), YuvMatrix::Bt709);
        assert_eq!(YuvMatrix::from_tag(None, 576), YuvMatrix::Bt601);
        assert_eq!(YuvMatrix::from_tag(Some("bt709"), 480), YuvMatrix::Bt709);
        assert_eq!(
            YuvMatrix::from_tag(Some("smpte170m"), 1080),
            YuvMatrix::Bt601
        );
        assert_eq!(
            YuvMatrix::from_tag(Some("bt2020nc"), 2160),
            YuvMatrix::Bt2020Ncl
        );
    }

    #[test]
    fn range_expansion_for_8_and_10_bits() {
        let limited8 = RangeCoefficients::new(ColorRange::Limited, 8, false);
        assert_close3(limited8.expand([16.0, 128.0, 128.0]), [0.0, 0.0, 0.0]);
        assert_close3(limited8.expand([235.0, 240.0, 16.0]), [1.0, 0.5, -0.5]);
        let limited10 = RangeCoefficients::new(ColorRange::Limited, 10, false);
        assert_close3(limited10.expand([64.0, 512.0, 512.0]), [0.0, 0.0, 0.0]);
        assert_close3(limited10.expand([940.0, 960.0, 64.0]), [1.0, 0.5, -0.5]);
        let full8 = RangeCoefficients::new(ColorRange::Full, 8, false);
        assert_close3(full8.expand([0.0, 128.0, 128.0]), [0.0, 0.0, 0.0]);
        assert!(close(full8.expand([255.0, 255.0, 1.0])[0], 1.0, 1e-7));
        assert!(close(
            full8.expand([255.0, 255.0, 1.0])[1],
            127.0 / 255.0,
            1e-7
        ));
        let full10 = RangeCoefficients::new(ColorRange::Full, 10, false);
        assert_close3(full10.expand([1023.0, 512.0, 512.0]), [1.0, 0.0, 0.0]);
        // RGB: every channel follows the luma rule.
        let rgb = RangeCoefficients::new(ColorRange::Full, 8, true);
        assert_close3(rgb.expand([255.0, 0.0, 51.0]), [1.0, 0.0, 0.2]);
        // Unknown range defaults.
        assert_eq!(
            ColorRange::from_tag(None, "yuv420p", false),
            ColorRange::Limited
        );
        assert_eq!(
            ColorRange::from_tag(None, "yuvj420p", false),
            ColorRange::Full
        );
        assert_eq!(ColorRange::from_tag(None, "rgb24", true), ColorRange::Full);
        assert_eq!(
            ColorRange::from_tag(Some("tv"), "yuvj420p", false),
            ColorRange::Limited
        );
    }

    fn params(
        matrix: Option<YuvMatrix>,
        range: ColorRange,
        transfer: TransferPath,
    ) -> ConvertParams {
        ConvertParams {
            matrix,
            range,
            transfer,
            rotation: Rotation::None,
            alpha: AlphaMode::Opaque,
            display_size: (2, 2),
        }
    }

    #[test]
    fn convert_pixel_paths() {
        let sdr = params(
            Some(YuvMatrix::Bt709),
            ColorRange::Limited,
            TransferPath::Sdr,
        );
        // Limited-range white, black and BT.709 red.
        assert_close3(convert_pixel([235.0, 128.0, 128.0], 8, &sdr), [1.0; 3]);
        assert_close3(convert_pixel([16.0, 128.0, 128.0], 8, &sdr), [0.0; 3]);
        let red = convert_pixel([63.0, 102.0, 240.0], 8, &sdr);
        assert!(red[0] > 0.99 && red[1] < 0.01 && red[2] < 0.01, "{red:?}");
        // Super-white is clipped.
        assert_eq!(convert_pixel([255.0, 128.0, 128.0], 8, &sdr), [1.0; 3]);

        // SDR BT.2020 greys stay grey (the matrix rows sum to 1).
        let wide = params(
            Some(YuvMatrix::Bt2020Ncl),
            ColorRange::Limited,
            TransferPath::SdrBt2020,
        );
        let grey = convert_pixel([502.0, 512.0, 512.0], 10, &wide);
        assert!(
            close(grey[0], grey[1], 1e-4) && close(grey[1], grey[2], 1e-4),
            "{grey:?}"
        );

        // HLG: 75 % grey is reference white, slightly compressed by the knee; black is black.
        let hlg = params(
            Some(YuvMatrix::Bt2020Ncl),
            ColorRange::Limited,
            TransferPath::Hlg,
        );
        let white = convert_pixel([64.0 + 0.75 * 876.0, 512.0, 512.0], 10, &hlg);
        assert!(white.iter().all(|&v| (0.85..1.0).contains(&v)), "{white:?}");
        assert_eq!(convert_pixel([64.0, 512.0, 512.0], 10, &hlg), [0.0; 3]);
        assert_close3(convert_pixel([940.0, 512.0, 512.0], 10, &hlg), [1.0; 3]);

        // PQ: the source peak lands on SDR white, mid-tones are not touched.
        let pq = params(
            Some(YuvMatrix::Bt2020Ncl),
            ColorRange::Full,
            TransferPath::Pq { peak_nits: 1000 },
        );
        let peak = convert_pixel([pq_inverse_eotf(1000.0) * 1023.0, 512.0, 512.0], 10, &pq);
        assert!(peak.iter().all(|&v| close(v, 1.0, 1e-3)), "{peak:?}");
        let mid = convert_pixel([pq_inverse_eotf(20.3) * 1023.0, 512.0, 512.0], 10, &pq);
        let expected = bt1886_encode(0.1);
        assert!(mid.iter().all(|&v| close(v, expected, 1e-4)), "{mid:?}");

        // RGB sources skip the matrix.
        let rgb = params(None, ColorRange::Full, TransferPath::Sdr);
        assert_close3(convert_pixel([255.0, 0.0, 51.0], 8, &rgb), [1.0, 0.0, 0.2]);
    }

    #[test]
    fn bt1886_round_trips() {
        for v in [0.0, 0.1, 0.5, 1.0] {
            assert!(close(bt1886_decode(bt1886_encode(v)), v, 1e-6));
        }
        assert_eq!(
            sdr_bt2020_to_bt709([1.0; 3]).map(|v| (v * 1e4).round()),
            [1e4; 3]
        );
    }
}
