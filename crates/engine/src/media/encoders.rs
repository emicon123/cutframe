//! Encoder discovery that never selects a GPL, nonfree or otherwise excluded encoder.
//!
//! Selection is allowlist-first: only encoders named in [`EncoderTarget::preference`] can ever be
//! chosen, in that order, and only if the linked FFmpeg has them. On top of that a hard
//! [`DENYLIST`] is applied to every name, so a denied encoder is never returned even if a non-LGPL
//! FFmpeg build that contains it is loaded at run time (the startup check refuses such builds in
//! release, but debug builds only warn).
//!
//! Excluded on purpose:
//! - GPL encoders: `libx264`, `libx264rgb`, `libx265`, `libxvid`, `libxavs`, `libxavs2`.
//! - Nonfree encoders: `libfdk_aac`.
//! - `libopenh264` (BSD-licensed code, but see review F28): Cisco's patent licence covers only
//!   Cisco's own binaries downloaded at run time, not a copy bundled in FFmpeg, so the bundled one
//!   would ship H.264 without a patent licence. There is therefore no software H.264 encoder; H.264
//!   uses the GPU encoders or Media Foundation (`h264_mf`, whose software encoder comes with Windows).
//!   The export design decides whether a software fallback is needed at all (F28).
//! - Experimental encoders (e.g. FFmpeg's native `opus`): never selected.
//!
//! Discovery only says what *may* be used. Hardware encoders are compiled into the build whether or
//! not the machine has the GPU, so the export pipeline must still try to open each candidate in
//! order and fall through on failure.

use std::ffi::CStr;

use rsmpeg::avcodec::AVCodec;
use rsmpeg::ffi;

/// Encoders that are never selected, whatever the preference lists say and whatever the linked
/// FFmpeg contains.
pub const DENYLIST: &[&str] = &[
    // GPL
    "libx264",
    "libx264rgb",
    "libx265",
    "libxvid",
    "libxavs",
    "libxavs2",
    // nonfree
    "libfdk_aac",
    // patent licence does not cover bundled builds (F28)
    "libopenh264",
];

/// What an export needs an encoder for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncoderTarget {
    H264,
    Hevc,
    Av1,
    Aac,
    Opus,
}

/// Where an encoder runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncoderBackend {
    /// A GPU encoder (NVENC, Quick Sync, AMF). Present in the build even without the GPU.
    Hardware,
    /// A Windows Media Foundation encoder: a hardware MFT if the GPU driver has one, otherwise the
    /// software encoder that ships with Windows.
    MediaFoundation,
    /// A CPU encoder in the FFmpeg build.
    Software,
}

/// An allowlisted encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncoderCandidate {
    /// FFmpeg encoder name, as passed to `avcodec_find_encoder_by_name`.
    pub name: &'static str,
    pub backend: EncoderBackend,
}

const fn hw(name: &'static str) -> EncoderCandidate {
    EncoderCandidate {
        name,
        backend: EncoderBackend::Hardware,
    }
}

const fn mf(name: &'static str) -> EncoderCandidate {
    EncoderCandidate {
        name,
        backend: EncoderBackend::MediaFoundation,
    }
}

const fn sw(name: &'static str) -> EncoderCandidate {
    EncoderCandidate {
        name,
        backend: EncoderBackend::Software,
    }
}

// Allowlists, best first: GPU encoders (NVIDIA, Intel, AMD), then Media Foundation, then
// licence-clean software encoders.
const H264: &[EncoderCandidate] = &[
    hw("h264_nvenc"),
    hw("h264_qsv"),
    hw("h264_amf"),
    mf("h264_mf"),
];
const HEVC: &[EncoderCandidate] = &[
    hw("hevc_nvenc"),
    hw("hevc_qsv"),
    hw("hevc_amf"),
    mf("hevc_mf"),
    // No software HEVC encoder: the feature catalog (F1.91) asks for hardware/OS encoders only.
];
const AV1: &[EncoderCandidate] = &[
    hw("av1_nvenc"),
    hw("av1_qsv"),
    hw("av1_amf"),
    // Both BSD-2-Clause, with the AOMedia patent licence.
    sw("libsvtav1"),
    sw("libaom-av1"),
];
const AAC: &[EncoderCandidate] = &[
    // FFmpeg's native encoder (LGPL), ahead of Media Foundation's.
    sw("aac"),
    mf("aac_mf"),
];
// BSD-3-Clause. FFmpeg's native `opus` encoder is experimental and not listed.
const OPUS: &[EncoderCandidate] = &[sw("libopus")];

impl EncoderTarget {
    pub const ALL: [EncoderTarget; 5] = [Self::H264, Self::Hevc, Self::Av1, Self::Aac, Self::Opus];

    /// The allowlist for this target, best first. Only these encoders can ever be selected.
    pub const fn preference(self) -> &'static [EncoderCandidate] {
        match self {
            Self::H264 => H264,
            Self::Hevc => HEVC,
            Self::Av1 => AV1,
            Self::Aac => AAC,
            Self::Opus => OPUS,
        }
    }
}

/// True if `name` must never be selected.
pub fn is_denied(name: &str) -> bool {
    DENYLIST.contains(&name)
}

/// The allowlisted encoders for `target` that are in `available`, in preference order, with the
/// denylist applied. Pure: `available` is the list of encoder names the linked FFmpeg offers.
pub fn rank<'a>(
    target: EncoderTarget,
    available: impl IntoIterator<Item = &'a str>,
) -> Vec<EncoderCandidate> {
    let available: Vec<&str> = available.into_iter().collect();
    target
        .preference()
        .iter()
        .filter(|candidate| !is_denied(candidate.name) && available.contains(&candidate.name))
        .copied()
        .collect()
}

/// Names of all non-experimental encoders in the linked FFmpeg, denied ones included (callers use
/// [`rank`] / [`candidates`], which filter them).
pub fn available_encoders() -> Vec<&'static str> {
    AVCodec::iterate()
        .filter(|codec| {
            // SAFETY: `codec` comes from `av_codec_iterate` and points to a static `AVCodec`.
            let encoder = unsafe { ffi::av_codec_is_encoder(codec.as_ptr()) } != 0;
            let experimental = codec.capabilities & ffi::AV_CODEC_CAP_EXPERIMENTAL as i32 != 0;
            encoder && !experimental
        })
        .filter_map(|codec| {
            // SAFETY: `AVCodec::name` is a static, NUL-terminated string owned by FFmpeg's codec
            // table, which lives as long as the process.
            let name: &'static CStr = unsafe { CStr::from_ptr(codec.name) };
            name.to_str().ok()
        })
        .collect()
}

/// The encoders the export may try for `target`, best first, from the linked FFmpeg.
pub fn candidates(target: EncoderTarget) -> Vec<EncoderCandidate> {
    rank(target, available_encoders())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encoders the pinned Windows LGPL build offers for these targets, plus GPL ones a GPL build
    /// would add.
    const WINDOWS_LGPL_BUILD: &[&str] = &[
        "libopenh264",
        "h264_amf",
        "h264_mf",
        "h264_nvenc",
        "h264_qsv",
        "hevc_amf",
        "hevc_mf",
        "hevc_nvenc",
        "hevc_qsv",
        "libkvazaar",
        "av1_nvenc",
        "av1_qsv",
        "av1_amf",
        "libsvtav1",
        "libaom-av1",
        "librav1e",
        "aac",
        "aac_mf",
        "libopus",
    ];
    const GPL_EXTRAS: &[&str] = &["libx264", "libx264rgb", "libx265", "libxvid", "libfdk_aac"];

    fn names(candidates: &[EncoderCandidate]) -> Vec<&'static str> {
        candidates.iter().map(|c| c.name).collect()
    }

    #[test]
    fn allowlist_and_denylist_are_disjoint() {
        for target in EncoderTarget::ALL {
            for candidate in target.preference() {
                assert!(!is_denied(candidate.name), "{target:?}: {}", candidate.name);
            }
        }
    }

    #[test]
    fn hardware_comes_first_then_media_foundation_then_software() {
        for target in EncoderTarget::ALL {
            let order: Vec<u8> = target
                .preference()
                .iter()
                .map(|c| match c.backend {
                    EncoderBackend::Hardware => 0,
                    EncoderBackend::MediaFoundation => 1,
                    EncoderBackend::Software => 2,
                })
                .collect();
            // AAC prefers FFmpeg's own encoder over Media Foundation's.
            if target != EncoderTarget::Aac {
                assert!(order.is_sorted(), "{target:?}: {order:?}");
            }
        }
    }

    #[test]
    fn ranks_by_preference_not_by_availability_order() {
        let available = ["h264_mf", "h264_amf", "h264_nvenc"];
        assert_eq!(
            names(&rank(EncoderTarget::H264, available)),
            ["h264_nvenc", "h264_amf", "h264_mf"]
        );
    }

    #[test]
    fn pinned_build_rankings() {
        let available = WINDOWS_LGPL_BUILD.iter().copied();
        assert_eq!(
            names(&rank(EncoderTarget::H264, available.clone())),
            ["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]
        );
        assert_eq!(
            names(&rank(EncoderTarget::Hevc, available.clone())),
            ["hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_mf"]
        );
        assert_eq!(
            names(&rank(EncoderTarget::Aac, available.clone())),
            ["aac", "aac_mf"]
        );
        assert_eq!(names(&rank(EncoderTarget::Opus, available)), ["libopus"]);
    }

    #[test]
    fn gpl_nonfree_and_openh264_are_never_selected_even_if_present() {
        let available: Vec<&str> = GPL_EXTRAS
            .iter()
            .chain(WINDOWS_LGPL_BUILD)
            .copied()
            .collect();
        for target in EncoderTarget::ALL {
            for candidate in rank(target, available.iter().copied()) {
                assert!(!is_denied(candidate.name), "{target:?}: {}", candidate.name);
            }
        }
        // With only denied encoders around, nothing is selected.
        for target in EncoderTarget::ALL {
            let denied = GPL_EXTRAS.iter().copied().chain(["libopenh264"]);
            assert_eq!(rank(target, denied), [], "{target:?}");
        }
    }

    #[test]
    fn unknown_encoders_are_never_selected() {
        assert_eq!(
            rank(EncoderTarget::H264, ["h264_vaapi", "h264_v4l2m2m"]),
            []
        );
    }

    /// Against the linked FFmpeg: the pinned LGPL build has none of the denied GPL/nonfree encoders,
    /// and every platform has FFmpeg's native AAC encoder.
    #[test]
    fn linked_ffmpeg_has_no_gpl_encoders() {
        let available = available_encoders();
        for denied in GPL_EXTRAS {
            assert!(
                !available.contains(denied),
                "{denied} is in the linked FFmpeg"
            );
        }
        assert_eq!(
            candidates(EncoderTarget::Aac).first().map(|c| c.name),
            Some("aac")
        );
        for target in EncoderTarget::ALL {
            for candidate in candidates(target) {
                assert!(!is_denied(candidate.name));
            }
        }
    }
}
