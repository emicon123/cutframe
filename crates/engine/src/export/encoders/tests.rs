use super::*;

use EncoderKind::*;
use VideoCodec::{H264, Hevc};

#[test]
fn candidate_order_per_codec() {
    assert_eq!(
        candidates(H264),
        [Nvenc, Qsv, Amf, MfHardware, MfSoftware, OpenH264]
    );
    assert_eq!(candidates(Hevc), [Nvenc, Qsv, Amf, MfHardware]);
    for codec in VideoCodec::ALL {
        for &kind in candidates(codec) {
            assert!(kind.encoder_name(codec).is_some(), "{kind:?} {codec:?}");
        }
    }
    assert_eq!(MfSoftware.encoder_name(Hevc), None);
    assert_eq!(OpenH264.encoder_name(Hevc), None);
    let hardware: Vec<_> = EncoderKind::ALL
        .into_iter()
        .filter(|k| k.is_hardware())
        .collect();
    assert_eq!(hardware, [Nvenc, Qsv, Amf, MfHardware]);
}

#[test]
fn encoders_env_parsing() {
    assert_eq!(EncoderFilter::parse(""), (EncoderFilter::default(), vec![]));
    assert_eq!(
        EncoderFilter::parse("openH264"),
        (EncoderFilter::only(&[OpenH264]), vec![])
    );
    assert_eq!(
        EncoderFilter::parse(" mfsoftware , OPENH264,openH264 "),
        (EncoderFilter::only(&[MfSoftware, OpenH264]), vec![])
    );
    assert_eq!(
        EncoderFilter::parse("nvenc,x264"),
        (EncoderFilter::only(&[Nvenc]), vec!["x264".to_owned()])
    );
    // nothing known: no limit
    assert_eq!(
        EncoderFilter::parse("x264, ,"),
        (EncoderFilter::default(), vec!["x264".to_owned()])
    );
    let filter = EncoderFilter::only(&[OpenH264, Qsv]);
    assert!(filter.allows(Qsv) && filter.allows(OpenH264) && !filter.allows(Nvenc));
    assert!(EncoderFilter::default().allows(Amf));
    for kind in EncoderKind::ALL {
        assert_eq!(
            EncoderFilter::parse(kind.key()).0,
            EncoderFilter::only(&[kind])
        );
    }
}

fn failed() -> EncoderError {
    EncoderError::Failed {
        encoder: "fake",
        stage: "open",
        detail: "no device".into(),
    }
}

/// A probe where NVENC fails, QSV needs `Safe` and everything else works.
fn fake_probe(kind: EncoderKind, _codec: VideoCodec) -> Result<OptionSet, EncoderError> {
    match kind {
        Nvenc => Err(failed()),
        Qsv => Ok(OptionSet::Safe),
        _ => Ok(OptionSet::Full),
    }
}

#[test]
fn selection_follows_the_table_with_a_fake_probe() {
    let report = ProbeReport::run_with(&EncoderFilter::default(), fake_probe);
    assert_eq!(report.entries.len(), 10);
    assert_eq!(report.entries[0].codec, H264);
    assert_eq!(report.entries[6].codec, Hevc);
    let kinds = |c: &[EncoderCandidate]| c.iter().map(|c| c.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds(&report.candidates(H264, None)),
        [Qsv, Amf, MfHardware, MfSoftware, OpenH264]
    );
    assert_eq!(
        report.candidates(H264, None)[0],
        EncoderCandidate {
            kind: Qsv,
            codec: H264,
            options: OptionSet::Safe
        }
    );
    assert_eq!(
        kinds(&report.candidates(Hevc, None)),
        [Qsv, Amf, MfHardware]
    );
    // explicit: only that kind, nothing if unavailable
    assert_eq!(kinds(&report.candidates(H264, Some(OpenH264))), [OpenH264]);
    assert_eq!(report.candidates(H264, Some(Nvenc)), []);
    assert_eq!(report.candidates(Hevc, Some(OpenH264)), []);
    assert!(!report.is_available(Nvenc, H264));
    assert!(report.is_available(MfHardware, Hevc));
}

#[test]
fn excluded_candidates_are_not_probed() {
    let mut probed = Vec::new();
    let report = ProbeReport::run_with(&EncoderFilter::only(&[MfSoftware, OpenH264]), |k, c| {
        probed.push((k, c));
        Ok(OptionSet::Full)
    });
    assert_eq!(probed, [(MfSoftware, H264), (OpenH264, H264)]);
    assert!(
        report
            .entries
            .iter()
            .filter(|e| !matches!(e.kind, MfSoftware | OpenH264))
            .all(|e| e.outcome == ProbeOutcome::Excluded)
    );
    assert_eq!(report.candidates(Hevc, None), []);
}

#[test]
fn safe_options_drop_b_frames_and_slow_features() {
    let safe_variants: Vec<_> = VideoCodec::ALL
        .into_iter()
        .flat_map(|codec| candidates(codec).iter().map(move |&kind| (kind, codec)))
        .filter(|&(kind, codec)| has_safe_variant(kind, codec))
        .collect();
    assert_eq!(
        safe_variants,
        [
            (Nvenc, H264),
            (Qsv, H264),
            (Amf, H264),
            (OpenH264, H264),
            (Nvenc, Hevc),
            (Qsv, Hevc),
        ]
    );
    let safe = |kind, codec| EncoderCandidate {
        kind,
        codec,
        options: OptionSet::Safe,
    };
    for codec in VideoCodec::ALL {
        for &kind in candidates(codec) {
            assert_eq!(b_frames(safe(kind, codec)), 0);
            let options = private_options(safe(kind, codec));
            assert!(!options.iter().any(|(name, _)| *name == c"multipass"));
            assert!(!options.contains(&(c"bf", c"2")));
        }
    }
}

/// Against the linked FFmpeg: every private option name and value of every candidate and option
/// set is accepted by its encoder (no hardware needed: the context is not opened).
#[test]
fn every_option_is_known_to_the_linked_encoders() {
    for codec in VideoCodec::ALL {
        for &kind in candidates(codec) {
            for options in [OptionSet::Full, OptionSet::Safe] {
                let candidate = EncoderCandidate {
                    kind,
                    codec,
                    options,
                };
                let (encoder, name) = find(candidate).unwrap();
                let mut context = AVCodecContext::new(&encoder);
                set_private_options(&mut context, candidate, name)
                    .unwrap_or_else(|err| panic!("{kind:?} {codec:?} {options:?}: {err}"));
                let formats = encoder.pix_fmts().unwrap_or_default();
                assert!(
                    formats.contains(&kind.pixel_format()),
                    "{name} takes {:?}",
                    kind.pixel_format()
                );
            }
        }
    }
}

#[test]
fn an_unknown_option_is_an_error() {
    let candidate = EncoderCandidate {
        kind: OpenH264,
        codec: H264,
        options: OptionSet::Full,
    };
    let (encoder, _) = find(candidate).unwrap();
    let mut context = AVCodecContext::new(&encoder);
    // SAFETY: as in `set_private_options`.
    let ret = unsafe {
        ffi::av_opt_set(
            (*context.as_mut_ptr()).priv_data,
            c"no_such_option".as_ptr(),
            c"1".as_ptr(),
            0,
        )
    };
    assert_eq!(ret, ffi::AVERROR_OPTION_NOT_FOUND);
}

#[test]
fn the_linked_ffmpeg_has_every_required_encoder_and_muxer() {
    assert_eq!(required_encoders_present(), Ok(()));
}

/// GPL and nonfree encoders a non-LGPL build would add; the pinned build has none of them.
#[test]
fn the_linked_ffmpeg_has_no_gpl_encoders() {
    for name in [
        c"libx264",
        c"libx264rgb",
        c"libx265",
        c"libxvid",
        c"libfdk_aac",
    ] {
        assert!(
            AVCodec::find_encoder_by_name(name).is_none(),
            "{name:?} is in the linked FFmpeg"
        );
    }
}

/// OpenH264 is in the build and runs on the CPU: it must work on every machine (CI included).
#[test]
fn openh264_probe_succeeds() {
    assert_eq!(probe(OpenH264, H264), Ok(OptionSet::Full));
    let encoder = open_encoder(
        EncoderCandidate {
            kind: OpenH264,
            codec: H264,
            options: OptionSet::Full,
        },
        &EncoderSettings::PROBE,
    )
    .unwrap();
    // SAFETY: reads plain fields of an open codec context.
    let (extradata, range, location) = unsafe {
        let raw = encoder.context.as_ptr();
        (
            (*raw).extradata_size,
            (*raw).color_range,
            (*raw).chroma_sample_location,
        )
    };
    assert!(extradata > 0, "global header for MP4");
    assert_eq!(range, ffi::AVCOL_RANGE_MPEG);
    assert_eq!(location, ffi::AVCHROMA_LOC_LEFT);
}

/// A kind without the hardware (or software HEVC) fails with an error, never a panic or a hang.
#[test]
fn unavailable_kinds_fail_cleanly() {
    assert_eq!(
        probe(MfSoftware, Hevc),
        Err(EncoderError::NoEncoderForCodec {
            kind: MfSoftware,
            codec: Hevc
        })
    );
    let impossible = EncoderSettings {
        width: 16_384,
        height: 16_384,
        ..EncoderSettings::PROBE
    };
    let err = open_encoder(
        EncoderCandidate {
            kind: OpenH264,
            codec: H264,
            options: OptionSet::Full,
        },
        &impossible,
    )
    .unwrap_err();
    assert!(
        matches!(err, EncoderError::Failed { stage: "open", .. }),
        "{err}"
    );
}

/// Hardware probe of this machine: prints which candidates pass. Run with
/// `cargo test -p engine --lib probe_this_machine -- --ignored --nocapture`.
#[test]
#[ignore = "probes the GPU encoders of this machine"]
fn probe_this_machine() {
    let _com = ComGuard::multithreaded();
    let started = Instant::now();
    let mut times = Vec::new();
    let report = ProbeReport::run_with(&EncoderFilter::default(), |kind, codec| {
        let one = Instant::now();
        let result = probe(kind, codec);
        times.push(one.elapsed());
        result
    });
    let elapsed = started.elapsed();
    for (entry, time) in report.entries.iter().zip(times) {
        println!(
            "{:?} {:?}: {:?} ({} ms)",
            entry.codec,
            entry.kind,
            entry.outcome,
            time.as_millis()
        );
    }
    println!("probe took {elapsed:?}");
    assert!(report.is_available(OpenH264, H264));
}
