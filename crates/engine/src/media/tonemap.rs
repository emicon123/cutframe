//! HDR→SDR tone mapping for thumbnails, on the CPU (design §5 "HDR previews").
//!
//! One simple operator for HLG and PQ: decode the transfer curve to display light relative to SDR
//! reference white (203 nits, ITU-R BT.2408), keep everything below a knee unchanged and roll the
//! highlights off so the peak lands on SDR white, convert BT.2020 primaries to BT.709 and encode
//! with the sRGB curve. Thumbnails only need to look right; they do not have to match the preview
//! pixel for pixel.
//!
//! TODO(0310): the render graph's tone map (WGSL) is designed in 0310 and built in 0320. When it
//! lands, these constants and the curve must come from the same definition, so posters and the
//! preview agree.

use app_core::media::{HdrInfo, HdrTransfer};
use rsmpeg::avutil::AVFrame;

use super::picture::{RgbImage, rgb48_rows};

/// SDR reference white in nits (BT.2408).
pub const SDR_WHITE_NITS: f32 = 203.0;
/// Nominal peak of HLG displays, and of PQ content without mastering metadata.
pub const DEFAULT_PEAK_NITS: f32 = 1000.0;
/// Relative light (1.0 = SDR white) below which the operator changes nothing.
pub const KNEE: f32 = 0.8;

/// Entries of the decode and encode lookup tables (12-bit code values).
const LUT_SIZE: usize = 4096;

/// BT.2020 to BT.709 primaries, linear light (BT.2087).
const BT2020_TO_BT709: [[f32; 3]; 3] = [
    [1.6605, -0.5876, -0.0728],
    [-0.1246, 1.1329, -0.0083],
    [-0.0182, -0.1006, 1.1187],
];

/// BT.2100 luminance weights for BT.2020 primaries.
const LUMA_2020: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// A tone map for one HDR source, with its lookup tables built.
#[derive(Debug, Clone)]
pub struct ToneMap {
    transfer: HdrTransfer,
    /// Peak of the source relative to SDR white.
    peak: f32,
    /// Code value (0..LUT_SIZE) → linear light: relative display light for PQ, scene light
    /// (0..=1) for HLG.
    decode: Vec<f32>,
    /// Linear 0..=1 → sRGB 8-bit.
    encode: Vec<u8>,
}

/// HLG inverse OETF (BT.2100): signal 0..=1 → scene light 0..=1.
fn hlg_inverse_oetf(signal: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    if signal <= 0.5 {
        signal * signal / 3.0
    } else {
        (((signal - C) / A).exp() + B) / 12.0
    }
}

/// PQ EOTF (SMPTE ST 2084): signal 0..=1 → nits.
fn pq_eotf(signal: f32) -> f32 {
    const M1: f32 = 0.159_301_76;
    const M2: f32 = 78.843_75;
    const C1: f32 = 0.835_937_5;
    const C2: f32 = 18.851_563;
    const C3: f32 = 18.6875;
    let p = signal.max(0.0).powf(1.0 / M2);
    ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1) * 10_000.0
}

fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// The highlight roll-off: identity below [`KNEE`], then a curve with slope 1 at the knee that
/// reaches 1.0 at `peak`. `x` and `peak` are relative to SDR white.
pub fn roll_off(x: f32, peak: f32) -> f32 {
    if x <= KNEE || peak <= 1.0 {
        return x.min(1.0);
    }
    let t = (x - KNEE) / (1.0 - KNEE);
    let tp = (peak - KNEE) / (1.0 - KNEE);
    let y = t * (1.0 + t / (tp * tp)) / (1.0 + t);
    (KNEE + (1.0 - KNEE) * y).min(1.0)
}

impl ToneMap {
    /// The tone map for `hdr`. Dolby Vision without a compatible base layer is mapped like PQ;
    /// its colours are wrong whatever we do (the media panel badges it).
    pub fn for_hdr(hdr: &HdrInfo) -> Self {
        let transfer = match hdr.transfer {
            HdrTransfer::Hlg => HdrTransfer::Hlg,
            HdrTransfer::Pq | HdrTransfer::DolbyVisionOnly => HdrTransfer::Pq,
        };
        let peak_nits = match transfer {
            HdrTransfer::Hlg => DEFAULT_PEAK_NITS,
            _ => hdr
                .max_cll_nits
                .or(hdr.mastering_max_nits)
                .map_or(DEFAULT_PEAK_NITS, |n| n as f32)
                .clamp(SDR_WHITE_NITS, 10_000.0),
        };
        let step = 1.0 / (LUT_SIZE - 1) as f32;
        let decode = (0..LUT_SIZE)
            .map(|i| {
                let signal = i as f32 * step;
                match transfer {
                    HdrTransfer::Hlg => hlg_inverse_oetf(signal),
                    _ => pq_eotf(signal) / SDR_WHITE_NITS,
                }
            })
            .collect();
        let encode = (0..LUT_SIZE)
            .map(|i| {
                (srgb_encode(i as f32 * step) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect();
        Self {
            transfer,
            peak: peak_nits / SDR_WHITE_NITS,
            decode,
            encode,
        }
    }

    /// Maps one pixel of non-linear BT.2020 RGB (`0..=65535`) to 8-bit sRGB.
    pub fn map(&self, rgb: [u16; 3]) -> [u8; 3] {
        let index = |v: u16| usize::from(v >> 4);
        let mut light = rgb.map(|v| self.decode[index(v)]);
        if self.transfer == HdrTransfer::Hlg {
            // HLG OOTF for a 1000-nit display: Fd = Lw * Ys^(gamma - 1) * E, gamma = 1.2.
            let ys = LUMA_2020[0] * light[0] + LUMA_2020[1] * light[1] + LUMA_2020[2] * light[2];
            let gain = DEFAULT_PEAK_NITS * ys.max(0.0).powf(0.2) / SDR_WHITE_NITS;
            light = light.map(|v| v * gain);
        }
        let max = light[0].max(light[1]).max(light[2]);
        if max > 0.0 {
            let scale = roll_off(max, self.peak) / max;
            light = light.map(|v| v * scale);
        }
        let m = &BT2020_TO_BT709;
        let bt709 =
            [0, 1, 2].map(|row| m[row][0] * light[0] + m[row][1] * light[1] + m[row][2] * light[2]);
        bt709.map(|v| {
            let i = (v.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32).round() as usize;
            self.encode[i]
        })
    }

    /// Tone-maps a whole RGB48LE frame.
    pub(crate) fn apply_rgb48(&self, frame: &AVFrame) -> RgbImage {
        let mut data = Vec::with_capacity(frame.width as usize * frame.height as usize * 3);
        for row in rgb48_rows(frame) {
            for &px in row.as_chunks::<3>().0 {
                data.extend_from_slice(&self.map(px));
            }
        }
        RgbImage {
            width: frame.width as u32,
            height: frame.height as u32,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(transfer: HdrTransfer, max_cll: Option<u32>) -> HdrInfo {
        HdrInfo {
            transfer,
            mastering_max_nits: None,
            max_cll_nits: max_cll,
            dolby_vision: None,
        }
    }

    #[test]
    fn curves_hit_their_reference_points() {
        assert!((pq_eotf(1.0) - 10_000.0).abs() < 1.0);
        assert!(pq_eotf(0.0).abs() < 1e-3);
        // PQ 0.58 is about 203 nits (BT.2408 reference white).
        assert!((pq_eotf(0.58) / SDR_WHITE_NITS - 1.0).abs() < 0.05);
        assert!((hlg_inverse_oetf(1.0) - 1.0).abs() < 1e-3);
        assert!((hlg_inverse_oetf(0.5) - 1.0 / 12.0).abs() < 1e-6);
    }

    #[test]
    fn roll_off_keeps_mid_tones_and_lands_the_peak_on_white() {
        assert_eq!(roll_off(0.5, 5.0), 0.5);
        assert_eq!(roll_off(KNEE, 5.0), KNEE);
        assert!((roll_off(5.0, 5.0) - 1.0).abs() < 1e-5);
        assert!(roll_off(2.0, 5.0) < 1.0 && roll_off(2.0, 5.0) > roll_off(1.0, 5.0));
        assert_eq!(roll_off(50.0, 5.0), 1.0);
    }

    #[test]
    fn hlg_reference_white_maps_near_sdr_white_and_black_stays_black() {
        let map = ToneMap::for_hdr(&hdr(HdrTransfer::Hlg, None));
        // 75 % HLG grey is reference white: light, but not clipped.
        let white = map.map([49_151; 3]);
        assert!(white.iter().all(|&v| (215..=250).contains(&v)), "{white:?}");
        assert_eq!(map.map([0; 3]), [0; 3]);
        assert_eq!(map.map([65_535; 3]), [255; 3]);
        // A saturated BT.2020 red stays red.
        let red = map.map([49_151, 0, 0]);
        assert!(red[0] > 150 && red[1] < 40 && red[2] < 40, "{red:?}");
    }

    #[test]
    fn pq_uses_the_content_peak() {
        let map = ToneMap::for_hdr(&hdr(HdrTransfer::Pq, Some(400)));
        assert!((map.peak - 400.0 / SDR_WHITE_NITS).abs() < 1e-6);
        // 400 nits (PQ ≈ 0.652) is the brightest the content gets: white.
        let peak = map.map([(0.6524 * 65_535.0) as u16; 3]);
        assert!(peak.iter().all(|&v| v >= 250), "{peak:?}");
    }
}
