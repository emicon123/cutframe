//! The linked FFmpeg: versions, licence, configure flags and the LGPL-only check.
//!
//! The check itself ([`FfmpegInfo::problems`]) is a pure function over strings and versions, so it is
//! unit-tested without FFmpeg. [`FfmpegInfo::linked`] reads the values from the DLLs actually loaded,
//! which may differ from the build the bindings were generated against (another FFmpeg on `PATH`,
//! DLLs swapped next to the exe).

use std::fmt;

use rsmpeg::avutil::AvVersion;
use rsmpeg::{avcodec, avdevice, avfilter, avformat, avutil, swresample, swscale};

/// Configure flags that make a build GPL or non-redistributable. Matched as whole tokens, because
/// the shipped LGPL build's configure line contains `--disable-libx264` and similar.
const FORBIDDEN_CONFIGURE_FLAGS: &[&str] = &["--enable-gpl", "--enable-nonfree"];

/// One FFmpeg library as linked at run time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryInfo {
    /// `avutil`, `avcodec`, ...
    pub name: &'static str,
    /// Version of the loaded library.
    pub linked: AvVersion,
    /// Version of the headers the bindings were generated against.
    pub built: AvVersion,
    /// `av*_license()`, e.g. `LGPL version 3 or later`.
    pub license: String,
    /// `av*_configuration()`: the configure command line of the build the library came from.
    pub configuration: String,
}

/// The linked FFmpeg as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInfo {
    /// `av_version_info()`, e.g. `n7.1.5-12-g1fdbca85aa-20260731`. Informative only, never parsed.
    pub version: String,
    pub libraries: Vec<LibraryInfo>,
}

/// Why a linked FFmpeg must not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfmpegProblem {
    /// The library reports a licence other than LGPL (GPL or "nonfree and unredistributable").
    NotLgpl {
        library: &'static str,
        license: String,
    },
    /// The library was configured with a flag that makes the build GPL or non-redistributable.
    ForbiddenConfigureFlag {
        library: &'static str,
        flag: &'static str,
    },
    /// The loaded library has another major version (ABI) than the bindings were generated for.
    AbiMismatch {
        library: &'static str,
        linked: AvVersion,
        built: AvVersion,
    },
}

impl fmt::Display for FfmpegProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLgpl { library, license } => {
                write!(f, "lib{library} is licensed \"{license}\", not LGPL")
            }
            Self::ForbiddenConfigureFlag { library, flag } => {
                write!(f, "lib{library} was configured with {flag}")
            }
            Self::AbiMismatch {
                library,
                linked,
                built,
            } => write!(
                f,
                "lib{library} {linked} is loaded, but the app was built against {built} (major version differs)"
            ),
        }
    }
}

/// Limits FFmpeg's own stderr logging to errors. Its default (`info`) prints a line for every
/// probed stream and every "misdetection possible" guess, which floods the console during an
/// import. Call once at startup; it is process-wide. Errors are still printed, and the app's own
/// FFmpeg licence log goes through `tracing`, so it is not affected.
pub fn quiet_logging() {
    // SAFETY: sets a global integer; safe to call from any thread at any time.
    unsafe { rsmpeg::ffi::av_log_set_level(rsmpeg::ffi::AV_LOG_ERROR as i32) };
}

/// True for the licence strings an LGPL build reports (`LGPL version 2.1 or later`,
/// `LGPL version 3 or later`). GPL builds report `GPL version ...`, nonfree builds
/// `nonfree and unredistributable`.
pub fn is_lgpl_license(license: &str) -> bool {
    license.trim_start().starts_with("LGPL version ")
}

/// The forbidden configure flags present in `configuration`, as whole whitespace-separated tokens.
pub fn forbidden_configure_flags(configuration: &str) -> Vec<&'static str> {
    FORBIDDEN_CONFIGURE_FLAGS
        .iter()
        .copied()
        .filter(|flag| configuration.split_whitespace().any(|token| token == *flag))
        .collect()
}

impl LibraryInfo {
    fn problems(&self) -> impl Iterator<Item = FfmpegProblem> + '_ {
        let licence = (!is_lgpl_license(&self.license)).then(|| FfmpegProblem::NotLgpl {
            library: self.name,
            license: self.license.clone(),
        });
        let flags = forbidden_configure_flags(&self.configuration)
            .into_iter()
            .map(|flag| FfmpegProblem::ForbiddenConfigureFlag {
                library: self.name,
                flag,
            });
        let abi = (self.linked.major != self.built.major).then_some(FfmpegProblem::AbiMismatch {
            library: self.name,
            linked: self.linked,
            built: self.built,
        });
        licence.into_iter().chain(flags).chain(abi)
    }
}

impl FfmpegInfo {
    /// Reads versions, licences and configure lines from the FFmpeg libraries loaded in this process.
    pub fn linked() -> Self {
        macro_rules! library {
            ($lib:ident) => {
                LibraryInfo {
                    name: stringify!($lib),
                    linked: $lib::version(),
                    built: $lib::VERSION,
                    license: $lib::license().to_string_lossy().into_owned(),
                    configuration: $lib::configuration().to_string_lossy().into_owned(),
                }
            };
        }
        Self {
            version: avutil::version_info().to_string_lossy().into_owned(),
            libraries: vec![
                library!(avutil),
                library!(avcodec),
                library!(avformat),
                library!(avfilter),
                library!(avdevice),
                library!(swresample),
                library!(swscale),
            ],
        }
    }

    /// Everything that makes this FFmpeg unusable for Cutframe. Empty for the pinned LGPL build.
    pub fn problems(&self) -> Vec<FfmpegProblem> {
        self.libraries
            .iter()
            .flat_map(LibraryInfo::problems)
            .collect()
    }

    /// The configure line of libavutil (all libraries of one build share it).
    pub fn configuration(&self) -> &str {
        self.libraries
            .first()
            .map_or("", |lib| lib.configuration.as_str())
    }

    /// `avutil 59.39.100, avcodec 61.19.101, ...` for logs.
    pub fn library_versions(&self) -> String {
        self.libraries
            .iter()
            .map(|lib| format!("{} {}", lib.name, lib.linked))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The licence of libavutil, e.g. `LGPL version 3 or later`.
    pub fn license(&self) -> &str {
        self.libraries
            .first()
            .map_or("", |lib| lib.license.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_logging_keeps_errors_only() {
        quiet_logging();
        // SAFETY: reads a global integer.
        let level = unsafe { rsmpeg::ffi::av_log_get_level() };
        assert_eq!(level, rsmpeg::ffi::AV_LOG_ERROR as i32);
    }

    const LGPL_CONFIGURE: &str = "--prefix=/ffbuild/prefix --enable-version3 --disable-debug \
        --enable-shared --disable-static --enable-libopenh264 --disable-libx264 --disable-libx265 \
        --disable-libxvid --disable-libfdk-aac";

    fn library(license: &str, configuration: &str) -> LibraryInfo {
        LibraryInfo {
            name: "avutil",
            linked: AvVersion::new(59, 39, 100),
            built: AvVersion::new(59, 39, 100),
            license: license.to_owned(),
            configuration: configuration.to_owned(),
        }
    }

    fn info(libraries: Vec<LibraryInfo>) -> FfmpegInfo {
        FfmpegInfo {
            version: "n7.1.5".to_owned(),
            libraries,
        }
    }

    #[test]
    fn lgpl_licences_are_accepted() {
        assert!(is_lgpl_license("LGPL version 3 or later"));
        assert!(is_lgpl_license("LGPL version 2.1 or later"));
    }

    #[test]
    fn gpl_and_nonfree_licences_are_rejected() {
        for license in [
            "GPL version 2 or later",
            "GPL version 3 or later",
            "nonfree and unredistributable",
            "",
        ] {
            assert!(!is_lgpl_license(license), "{license}");
        }
    }

    #[test]
    fn disable_flags_are_not_mistaken_for_enable_flags() {
        assert!(forbidden_configure_flags(LGPL_CONFIGURE).is_empty());
        // Substrings of other flags do not count either.
        assert!(forbidden_configure_flags("--enable-gpl-ish --x--enable-nonfree").is_empty());
    }

    #[test]
    fn gpl_and_nonfree_configure_flags_are_found() {
        let configure = format!("{LGPL_CONFIGURE} --enable-gpl --enable-libx264 --enable-nonfree");
        assert_eq!(
            forbidden_configure_flags(&configure),
            ["--enable-gpl", "--enable-nonfree"]
        );
    }

    #[test]
    fn pinned_lgpl_build_has_no_problems() {
        let info = info(vec![library("LGPL version 3 or later", LGPL_CONFIGURE)]);
        assert_eq!(info.problems(), []);
    }

    #[test]
    fn gpl_build_is_reported() {
        let info = info(vec![library(
            "GPL version 3 or later",
            "--enable-gpl --enable-version3 --enable-libx264",
        )]);
        assert_eq!(
            info.problems(),
            [
                FfmpegProblem::NotLgpl {
                    library: "avutil",
                    license: "GPL version 3 or later".to_owned(),
                },
                FfmpegProblem::ForbiddenConfigureFlag {
                    library: "avutil",
                    flag: "--enable-gpl",
                },
            ]
        );
    }

    #[test]
    fn one_gpl_library_among_lgpl_ones_is_reported() {
        let mut avcodec = library("GPL version 2 or later", "--enable-gpl");
        avcodec.name = "avcodec";
        let info = info(vec![
            library("LGPL version 3 or later", LGPL_CONFIGURE),
            avcodec,
        ]);
        let problems = info.problems();
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().all(|p| matches!(
            p,
            FfmpegProblem::NotLgpl {
                library: "avcodec",
                ..
            } | FfmpegProblem::ForbiddenConfigureFlag {
                library: "avcodec",
                ..
            }
        )));
    }

    #[test]
    fn major_version_mismatch_is_reported_minor_is_not() {
        let mut newer_minor = library("LGPL version 3 or later", LGPL_CONFIGURE);
        newer_minor.linked = AvVersion::new(59, 40, 0);
        assert_eq!(info(vec![newer_minor]).problems(), []);

        let mut other_major = library("LGPL version 3 or later", LGPL_CONFIGURE);
        other_major.linked = AvVersion::new(60, 8, 100);
        assert_eq!(
            info(vec![other_major]).problems(),
            [FfmpegProblem::AbiMismatch {
                library: "avutil",
                linked: AvVersion::new(60, 8, 100),
                built: AvVersion::new(59, 39, 100),
            }]
        );
    }

    /// The "in CI" licence assertion: the FFmpeg this test binary actually loaded (the pinned DLLs on
    /// `PATH`, set up by `scripts/fetch-ffmpeg.ps1`) is LGPL, has no GPL/nonfree configure flags and
    /// matches the bindings' ABI. `cargo test` fails otherwise.
    #[test]
    fn linked_ffmpeg_is_lgpl() {
        let info = FfmpegInfo::linked();
        assert_eq!(info.libraries.len(), 7);
        assert!(info.problems().is_empty(), "{:#?}", info.problems());
        assert!(info.license().starts_with("LGPL version"));
        assert!(
            info.configuration().contains("--enable-version3"),
            "the pinned build is LGPL v3: {}",
            info.configuration()
        );
    }
}
