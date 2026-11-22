//! Startup check of the linked FFmpeg: LGPL only (CLAUDE.md rule 7, review F26).
//!
//! Logs the FFmpeg version, licence and configure flags, then refuses to start a release build whose
//! FFmpeg is not LGPL (or was configured with `--enable-gpl` / `--enable-nonfree`, or has another ABI
//! than the app was built against). Debug builds log the same problems loudly and continue, so a
//! developer with a stray FFmpeg on `PATH` can still work. The check logic lives in
//! `engine::media::ffmpeg` and is unit-tested there; `cargo test` also asserts it against the linked
//! build.

use engine::media::FfmpegInfo;

/// Process exit code when a release build refuses to start because of the FFmpeg build. Also listed
/// in the smoke-test exit codes (`smoke_test.rs`).
pub const EXIT_CODE: i32 = 6;

/// Runs the check. Call once at startup, after tracing is initialised and before anything else
/// uses FFmpeg. Exits the process with [`EXIT_CODE`] in release builds if the build is not allowed.
pub fn check() {
    // FFmpeg's own stderr log: errors only (the licence log below goes through tracing).
    engine::media::ffmpeg::quiet_logging();
    let info = FfmpegInfo::linked();
    tracing::info!(
        version = %info.version,
        license = %info.license(),
        libraries = %info.library_versions(),
        "FFmpeg loaded"
    );
    tracing::info!(configuration = %info.configuration(), "FFmpeg configure flags");

    let problems = info.problems();
    if problems.is_empty() {
        return;
    }
    for problem in &problems {
        tracing::error!(%problem, "FFmpeg build not allowed: Cutframe ships LGPL FFmpeg only");
    }

    if cfg!(debug_assertions) {
        tracing::warn!(
            "DEBUG BUILD CONTINUES WITH A DISALLOWED FFMPEG. Release builds refuse to start with it. \
             Run `pnpm setup:ffmpeg` and make sure .ffmpeg\\bin comes first on PATH."
        );
    } else {
        // Release builds on Windows have no console; the log carries the details.
        eprintln!(
            "Cutframe cannot start: the FFmpeg libraries next to the app are not the LGPL build it \
             ships with ({}). Reinstall Cutframe.",
            problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
        std::process::exit(EXIT_CODE);
    }
}
