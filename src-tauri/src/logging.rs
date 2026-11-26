//! Logging: `tracing` to stdout and to rolling log files in the app log folder (F0.09).
//!
//! The log folder is Tauri's `app_log_dir()` (`%LOCALAPPDATA%\app.cutframe.editor\logs` on
//! Windows). It is resolved here, before Tauri starts, so the FFmpeg check and early panics reach
//! the file too; `setup` compares it with Tauri's own resolution. Files rotate daily
//! (`cutframe.YYYY-MM-DD.log`) and only the newest [`MAX_LOG_FILES`] are kept.
//!
//! Writes to the file are synchronous (no background writer thread), so the line a panic hook
//! writes is on disk before the process dies. The app logs little; the audio thread never logs.

use std::path::{Path, PathBuf};

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// File names are `<prefix>.<date>.<suffix>`.
pub const LOG_FILE_PREFIX: &str = "cutframe";
pub const LOG_FILE_SUFFIX: &str = "log";
/// Daily files kept; older ones are deleted when a new file starts.
pub const MAX_LOG_FILES: usize = 7;
/// Used when `RUST_LOG` is not set (or invalid): info for everything, quieter GPU internals.
pub const DEFAULT_FILTER: &str = "info,wgpu_core=warn,wgpu_hal=warn,naga=warn";

/// The log folder under the per-user local data folder, as Tauri's `app_log_dir()` resolves it on
/// Windows (and Linux).
pub fn log_dir_in(local_data_dir: &Path, identifier: &str) -> PathBuf {
    local_data_dir.join(identifier).join("logs")
}

/// The app's log folder, or `None` when the system has no local data folder.
pub fn log_dir(identifier: &str) -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| log_dir_in(&dir, identifier))
}

/// The folder the log file is written to (`None`: stdout only). Managed Tauri state.
#[derive(Debug, Clone)]
pub struct LogDir(pub Option<PathBuf>);

/// The `RUST_LOG` filter, or [`DEFAULT_FILTER`].
fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

/// The daily-rotating file writer in `dir` (created if missing).
pub fn file_appender(
    dir: &Path,
) -> Result<RollingFileAppender, tracing_appender::rolling::InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(dir)
}

/// Installs the global subscriber (stdout, plus the log file when `dir` can be opened) and the
/// panic hook. Returns the folder the file is written to, `None` if there is no file. Call once,
/// first thing in `run`.
pub fn init(dir: Option<PathBuf>) -> Option<PathBuf> {
    let (file, failure) = match dir.as_deref().map(file_appender) {
        Some(Ok(appender)) => (Some(appender), None),
        Some(Err(err)) => (None, Some(err)),
        None => (None, None),
    };
    let file_layer = file.map(|appender| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(appender)
    });
    tracing_subscriber::registry()
        .with(env_filter())
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .init();
    install_panic_hook();

    let written = match (dir, failure) {
        (Some(dir), None) => Some(dir),
        (Some(dir), Some(err)) => {
            tracing::warn!(%err, dir = %dir.display(), "cannot open the log file; logging to stdout only");
            None
        }
        (None, _) => {
            tracing::warn!("no local app data folder; logging to stdout only");
            None
        }
    };
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        profile = build_profile_name(),
        commit = option_env!("CUTFRAME_COMMIT").unwrap_or("unknown"),
        log_dir = ?written,
        "Cutframe starting"
    );
    written
}

fn build_profile_name() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Logs every panic (message, location, thread, backtrace) through `tracing`, then runs the
/// previous hook (which prints to stderr).
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            target: "panic",
            thread = thread.name().unwrap_or("<unnamed>"),
            location = %info.location().map_or_else(String::new, ToString::to_string),
            %backtrace,
            "panic: {}",
            panic_message(info.payload())
        );
        previous(info);
    }));
}

/// The text of a panic payload (`panic!("...")` gives a `&str` or a `String`).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[test]
    fn log_dir_matches_tauri_app_log_dir_layout() {
        assert_eq!(
            log_dir_in(
                Path::new("C:/Users/me/AppData/Local"),
                "app.cutframe.editor"
            ),
            Path::new("C:/Users/me/AppData/Local/app.cutframe.editor/logs")
        );
    }

    #[test]
    fn log_dir_uses_the_identifier_from_tauri_conf() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let identifier = config["identifier"].as_str().unwrap();
        if let Some(dir) = log_dir(identifier) {
            assert!(dir.ends_with(Path::new(identifier).join("logs")), "{dir:?}");
        }
    }

    #[test]
    fn default_filter_parses() {
        EnvFilter::try_new(DEFAULT_FILTER).expect("valid filter");
    }

    #[test]
    fn appender_writes_a_dated_file_with_the_prefix_and_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let appender = file_appender(&logs).expect("appender");
        let mut writer = appender.make_writer();
        writer.write_all(b"hello\n").unwrap();
        writer.flush().unwrap();
        drop(writer);

        let names: Vec<String> = std::fs::read_dir(&logs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let name = &names[0];
        assert!(
            name.starts_with("cutframe.") && name.ends_with(".log"),
            "{name}"
        );
        let content = std::fs::read_to_string(logs.join(name)).unwrap();
        assert_eq!(content, "hello\n");
    }

    #[test]
    fn panic_payloads_become_text() {
        let text: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(&*text), "boom");
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted 1"));
        assert_eq!(panic_message(&*owned), "formatted 1");
        let other: Box<dyn std::any::Any + Send> = Box::new(42_u32);
        assert_eq!(panic_message(&*other), "<non-string panic payload>");
    }
}
