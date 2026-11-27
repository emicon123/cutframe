//! Output file rules (export.md §5.2): which paths an export may write, the partial file it
//! writes first, and the state of an existing target.
//!
//! [`validate_output_path`] and [`is_same_file_path`] are pure (no file-system access beyond
//! `std::path::absolute`, which only normalises); [`folder_exists`] and [`target_state`] touch
//! the file system and belong on a worker, not in an IPC handler.

use std::path::{Path, PathBuf};

/// The extension every video export gets; appended when the name has none.
pub const VIDEO_EXTENSION: &str = "mp4";

/// Inserted before the extension of the file an export writes until it is complete.
pub const PARTIAL_MARKER: &str = "cutframe-partial";

/// Why a path cannot be an export target. Every variant is `invalidPath` for the UI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputPathError {
    #[error("the output path is not absolute")]
    NotAbsolute,
    #[error("the output path has no file name")]
    NoFileName,
    #[error("the file name contains a reserved character")]
    ReservedCharacter,
    #[error("the file name is a reserved device name")]
    ReservedName,
    #[error("the file name ends in a dot or a space")]
    TrailingDotOrSpace,
    #[error("the extension .{0} is not allowed for this export")]
    Extension(String),
}

/// Checks an output path from the UI and returns it with the extension filled in: `.mp4` is
/// appended when the name has none, and any other extension than `allowed` is refused. Does not
/// look at the file system (see [`folder_exists`] and [`target_state`]).
pub fn validate_output_path(raw: &str, allowed: &[&str]) -> Result<PathBuf, OutputPathError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(OutputPathError::NotAbsolute);
    }
    // The last segment as typed (`Path::components` would drop a trailing `.`); `C:\`, `..` and
    // `.` have no file name.
    let name = raw.rsplit(['\\', '/']).next().unwrap_or_default();
    if matches!(name, "" | "." | "..") {
        return Err(OutputPathError::NoFileName);
    }
    check_file_name(name)?;
    match Path::new(name).extension().and_then(|e| e.to_str()) {
        None => {
            let default = allowed.first().copied().unwrap_or(VIDEO_EXTENSION);
            Ok(path.with_file_name(format!("{name}.{default}")))
        }
        Some(ext) if allowed.iter().any(|a| a.eq_ignore_ascii_case(ext)) => Ok(path.to_path_buf()),
        Some(ext) => Err(OutputPathError::Extension(ext.to_owned())),
    }
}

/// The Windows file-name rules: no `<>:"/\|?*` or control characters, no reserved device name
/// (`CON`, `NUL`, `COM1`, ... with any extension), no trailing dot or space.
fn check_file_name(name: &str) -> Result<(), OutputPathError> {
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return Err(OutputPathError::ReservedCharacter);
    }
    if name.ends_with(['.', ' ']) {
        return Err(OutputPathError::TrailingDotOrSpace);
    }
    // Windows ignores everything from the first dot (and trailing spaces before it) when it
    // matches device names: `con.mp4` and `NUL .txt` are devices too.
    let base = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    if is_reserved_device_name(base) {
        return Err(OutputPathError::ReservedName);
    }
    Ok(())
}

fn is_reserved_device_name(base: &str) -> bool {
    const PLAIN: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if PLAIN.iter().any(|r| r.eq_ignore_ascii_case(base)) {
        return true;
    }
    // COM0-9 and LPT0-9, also with the superscript digits Windows treats as digits.
    let mut chars = base.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let rest: Vec<char> = chars.collect();
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && rest.len() == 1
        && (rest[0].is_ascii_digit() || matches!(rest[0], '¹' | '²' | '³'))
}

/// True if `a` and `b` name the same file on Windows: equal after `std::path::absolute`
/// (which resolves `.` and `..` and unifies separators), ignoring case.
pub fn is_same_file_path(a: &Path, b: &Path) -> bool {
    let normal = |p: &Path| {
        std::path::absolute(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase()
    };
    normal(a) == normal(b)
}

/// `<stem>.cutframe-partial.<ext>` next to `target`: same folder, so the final rename stays on
/// one volume and is atomic.
pub fn partial_path(target: &Path) -> PathBuf {
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = target.extension().map_or_else(
        || VIDEO_EXTENSION.to_owned(),
        |e| e.to_string_lossy().into_owned(),
    );
    target.with_file_name(format!("{stem}.{PARTIAL_MARKER}.{ext}"))
}

/// True if the folder `target` goes into exists.
pub fn folder_exists(target: &Path) -> bool {
    target.parent().is_some_and(Path::is_dir)
}

/// What is at the target path now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetState {
    /// Nothing: the export creates it.
    Free,
    /// A file that can be replaced (needs `overwrite`).
    Exists,
    /// A file another program holds open (a player), or one that cannot be written.
    Locked,
    /// A folder or something else that is not a file.
    NotAFile,
}

/// Looks at `target`: an existing file is opened for writing with no sharing and closed again
/// (without truncating it), so a file open in a player shows up as [`TargetState::Locked`].
pub fn target_state(target: &Path) -> TargetState {
    match std::fs::symlink_metadata(target) {
        Err(_) => TargetState::Free,
        Ok(meta) if !meta.is_file() => TargetState::NotAFile,
        Ok(_) => {
            if open_exclusive(target).is_ok() {
                TargetState::Exists
            } else {
                TargetState::Locked
            }
        }
    }
}

fn open_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MP4: &[&str] = &[VIDEO_EXTENSION];

    #[test]
    fn mp4_is_appended_and_other_extensions_are_refused() {
        assert_eq!(
            validate_output_path(r"C:\Videos\My vlog", MP4),
            Ok(PathBuf::from(r"C:\Videos\My vlog.mp4"))
        );
        assert_eq!(
            validate_output_path(r"C:\Videos\clip.mp4", MP4),
            Ok(PathBuf::from(r"C:\Videos\clip.mp4"))
        );
        assert_eq!(
            validate_output_path(r"C:\Videos\clip.MP4", MP4),
            Ok(PathBuf::from(r"C:\Videos\clip.MP4"))
        );
        for bad in ["exe", "dll", "mov", "bat", "5"] {
            assert_eq!(
                validate_output_path(&format!(r"C:\Videos\clip.{bad}"), MP4),
                Err(OutputPathError::Extension(bad.into())),
                "{bad}"
            );
        }
        // 0400's targets pass their own list
        assert_eq!(
            validate_output_path(r"C:\Music\mix", &["wav", "mp3"]),
            Ok(PathBuf::from(r"C:\Music\mix.wav"))
        );
    }

    #[test]
    fn paths_must_be_absolute_with_a_file_name() {
        assert_eq!(
            validate_output_path(r"Videos\clip.mp4", MP4),
            Err(OutputPathError::NotAbsolute)
        );
        assert_eq!(
            validate_output_path(r"\clip.mp4", MP4),
            Err(OutputPathError::NotAbsolute)
        );
        assert_eq!(
            validate_output_path("clip", MP4),
            Err(OutputPathError::NotAbsolute)
        );
        for no_name in [r"C:\", r"C:\Videos\", r"C:\Videos\..", r"C:\Videos\."] {
            assert_eq!(
                validate_output_path(no_name, MP4),
                Err(OutputPathError::NoFileName),
                "{no_name}"
            );
        }
        assert_eq!(
            validate_output_path(r"\\server\share\clip.mp4", MP4),
            Ok(PathBuf::from(r"\\server\share\clip.mp4"))
        );
    }

    #[test]
    fn reserved_characters_and_names_are_refused() {
        for name in [
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "a:b",
            "tab\there",
            "nul\0",
        ] {
            assert_eq!(
                validate_output_path(&format!(r"C:\Videos\{name}.mp4"), MP4),
                Err(OutputPathError::ReservedCharacter),
                "{name:?}"
            );
        }
        for name in [
            "CON",
            "con",
            "Nul",
            "PRN",
            "aux",
            "COM1",
            "com9",
            "LPT1",
            "lpt0",
            "COM¹",
            "CONIN$",
            "con.mp4",
            "NUL .mp4",
            "com1.backup.mp4",
        ] {
            assert_eq!(
                validate_output_path(&format!(r"C:\Videos\{name}"), MP4),
                Err(OutputPathError::ReservedName),
                "{name}"
            );
        }
        for name in ["clip.", "clip ", "clip.mp4.", "clip.mp4 "] {
            assert_eq!(
                validate_output_path(&format!(r"C:\Videos\{name}"), MP4),
                Err(OutputPathError::TrailingDotOrSpace),
                "{name:?}"
            );
        }
        // Names that only look like devices are fine.
        for name in ["CONSOLE", "COM10", "LPT", "Con game", "nul-take"] {
            assert!(
                validate_output_path(&format!(r"C:\Videos\{name}.mp4"), MP4).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn same_file_path_ignores_case_and_dot_segments() {
        let a = Path::new(r"C:\Footage\Trip\IMG_0042.MOV");
        assert!(is_same_file_path(
            a,
            Path::new(r"c:\footage\trip\img_0042.mov")
        ));
        assert!(is_same_file_path(
            a,
            Path::new(r"C:\Footage\Other\..\Trip\.\IMG_0042.MOV")
        ));
        assert!(is_same_file_path(
            a,
            Path::new("C:/Footage/Trip/IMG_0042.MOV")
        ));
        assert!(is_same_file_path(
            a,
            Path::new(r"\\?\C:\Footage\Trip\IMG_0042.MOV")
        ));
        assert!(!is_same_file_path(
            a,
            Path::new(r"C:\Footage\Trip\IMG_0043.MOV")
        ));
    }

    #[test]
    fn partial_file_sits_next_to_the_target() {
        assert_eq!(
            partial_path(Path::new(r"C:\Videos\My vlog.mp4")),
            PathBuf::from(r"C:\Videos\My vlog.cutframe-partial.mp4")
        );
        assert_eq!(
            partial_path(Path::new(r"C:\Videos\v1.2 final.mp4")),
            PathBuf::from(r"C:\Videos\v1.2 final.cutframe-partial.mp4")
        );
    }

    #[test]
    fn target_state_sees_free_existing_locked_and_folders() {
        let dir = std::env::temp_dir().join(format!("cutframe-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("clip.mp4");
        std::fs::remove_file(&target).ok();

        assert!(folder_exists(&target));
        assert!(!folder_exists(&dir.join("missing").join("clip.mp4")));
        assert_eq!(target_state(&target), TargetState::Free);
        assert_eq!(target_state(&dir), TargetState::NotAFile);

        std::fs::write(&target, b"old").unwrap();
        assert_eq!(target_state(&target), TargetState::Exists);
        assert_eq!(std::fs::read(&target).unwrap(), b"old", "not truncated");

        #[cfg(windows)]
        {
            let held = open_exclusive(&target).unwrap();
            assert_eq!(target_state(&target), TargetState::Locked);
            drop(held);
            assert_eq!(target_state(&target), TargetState::Exists);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
