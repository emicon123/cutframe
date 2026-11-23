//! HDR→SDR tone mapping for thumbnails, on the CPU (design §5 "HDR previews").
//!
//! The operator is the render graph's ([`crate::color::HdrToneMap`], render-graph.md §5.2 step 5):
//! PQ or HLG to display light, the BT.2390 EETF on `max(R, G, B)` from the source peak to SDR
//! reference white (203 nits), BT.2020 to BT.709, clip and BT.1886 encoding. Posters therefore show
//! the same colours as the preview, although nothing compares them pixel for pixel: swscale does
//! the Y'CbCr matrix and scaling here, the convert shader in the preview.

use app_core::media::HdrInfo;
use rsmpeg::avutil::AVFrame;

use super::picture::{RgbImage, rgb48_rows};
use crate::color::HdrToneMap;

/// A tone map for one HDR source.
#[derive(Debug, Clone, Copy)]
pub struct ToneMap {
    inner: HdrToneMap,
}

impl ToneMap {
    /// The tone map for `hdr`. Dolby Vision without a compatible base layer is mapped like PQ;
    /// its colours are wrong whatever we do (the media panel badges it).
    pub fn for_hdr(hdr: &HdrInfo) -> Self {
        Self {
            inner: HdrToneMap::for_hdr(hdr),
        }
    }

    /// Maps one pixel of non-linear BT.2020 RGB (`0..=65535`) to 8-bit R'G'B' (BT.709, BT.1886).
    pub fn map(&self, rgb: [u16; 3]) -> [u8; 3] {
        let signal = rgb.map(|v| f32::from(v) / 65535.0);
        self.inner
            .map(signal)
            .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
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
    use app_core::media::HdrTransfer;

    use super::*;
    use crate::color::pq_inverse_eotf;

    fn hdr(transfer: HdrTransfer, max_cll: Option<u32>) -> HdrInfo {
        HdrInfo {
            transfer,
            mastering_max_nits: None,
            max_cll_nits: max_cll,
            dolby_vision: None,
        }
    }

    fn code(signal: f32) -> u16 {
        (signal * 65535.0).round() as u16
    }

    #[test]
    fn hlg_reference_white_maps_near_sdr_white_and_black_stays_black() {
        let map = ToneMap::for_hdr(&hdr(HdrTransfer::Hlg, None));
        // 75 % HLG grey is reference white: light (the EETF's knee compresses it slightly), but
        // not clipped.
        let white = map.map([code(0.75); 3]);
        assert!(white.iter().all(|&v| (215..=245).contains(&v)), "{white:?}");
        assert_eq!(map.map([0; 3]), [0; 3]);
        assert_eq!(map.map([65_535; 3]), [255; 3]);
        // A saturated BT.2020 red stays red.
        let red = map.map([code(0.75), 0, 0]);
        assert!(red[0] > 150 && red[1] < 40 && red[2] < 40, "{red:?}");
    }

    #[test]
    fn pq_uses_the_content_peak() {
        let map = ToneMap::for_hdr(&hdr(HdrTransfer::Pq, Some(400)));
        // 400 nits is the brightest the content gets: white.
        let peak = map.map([code(pq_inverse_eotf(400.0)); 3]);
        assert!(peak.iter().all(|&v| v >= 254), "{peak:?}");
        // Below the knee the light is kept: 20.3 nits is 10 % of SDR white, 0.1^(1/2.4) = 0.383.
        let dim = map.map([code(pq_inverse_eotf(20.3)); 3]);
        assert!(dim.iter().all(|&v| (97..=98).contains(&v)), "{dim:?}");
        // A 1000-nit source keeps headroom above 400 nits, so it shows them darker.
        let bright = ToneMap::for_hdr(&hdr(HdrTransfer::Pq, Some(1000)));
        let below_peak = bright.map([code(pq_inverse_eotf(400.0)); 3]);
        assert!(below_peak[0] < peak[0] - 2, "{below_peak:?}");
    }
}
