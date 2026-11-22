//! Single instance and the project path on the command line (design `platform-integration.md`
//! §4). Explorer starts `cutframe.exe "<path>"` for a double-clicked project file.

use std::path::{Path, PathBuf};

/// Debug builds use the single-instance plugin only with `CUTFRAME_SINGLE_INSTANCE=1`: `pnpm dev`
/// shares the identifier with an installed app and would otherwise forward to it and quit.
pub const SINGLE_INSTANCE_ENV: &str = "CUTFRAME_SINGLE_INSTANCE";

/// The project path in a process's arguments: skips `args[0]` (the executable) and every
/// `--flag`, takes the first remaining argument and joins a relative one onto `cwd` (the
/// launching process's working directory). No extension or existence check: `open_project`
/// decides by content and reports errors.
pub fn project_path_from_args(args: &[String], cwd: &Path) -> Option<PathBuf> {
    let arg = args.iter().skip(1).find(|arg| !arg.starts_with("--"))?;
    Some(cwd.join(arg))
}

/// Whether to register the single-instance plugin: never for `--smoke-test` (a CI run must not be
/// forwarded to a running app and exit 0), always in release builds, and in debug builds only
/// with [`SINGLE_INSTANCE_ENV`]` = 1`.
pub fn single_instance_enabled(smoke_test: bool, debug_build: bool, env: Option<&str>) -> bool {
    !smoke_test && (!debug_build || env == Some("1"))
}

/// This process's arguments, lossy for non-Unicode ones (as the plugin passes a second
/// instance's).
pub fn own_args() -> Vec<String> {
    std::env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn cwd() -> PathBuf {
        std::env::temp_dir().join("launch-cwd")
    }

    #[test]
    fn no_arguments_or_flags_only_give_no_path() {
        assert_eq!(project_path_from_args(&[], &cwd()), None);
        assert_eq!(
            project_path_from_args(&args(&["cutframe.exe"]), &cwd()),
            None
        );
        assert_eq!(
            project_path_from_args(&args(&["cutframe.exe", "--smoke-test", "--x=1"]), &cwd()),
            None
        );
    }

    #[test]
    fn skips_flags_and_takes_the_first_path() {
        let absolute = cwd().join("a.cutframe");
        let absolute = absolute.to_str().unwrap();
        assert_eq!(
            project_path_from_args(
                &args(&["cutframe.exe", "--smoke-test", absolute, "second.cutframe"]),
                Path::new("/elsewhere"),
            ),
            Some(PathBuf::from(absolute))
        );
    }

    #[test]
    fn relative_paths_are_joined_onto_the_given_cwd() {
        assert_eq!(
            project_path_from_args(&args(&["cutframe.exe", "v/a.cutframe"]), &cwd()),
            Some(cwd().join("v/a.cutframe"))
        );
    }

    #[test]
    fn spaces_and_non_ascii_survive() {
        let name = "Café — 東京 trip 2026.cutframe";
        let path = cwd().join("My Projects").join(name);
        assert_eq!(
            project_path_from_args(
                &args(&["cutframe.exe", path.to_str().unwrap()]),
                Path::new("/elsewhere"),
            ),
            Some(path)
        );
    }

    #[test]
    fn single_instance_registration_rules() {
        // Release builds.
        assert!(single_instance_enabled(false, false, None));
        assert!(!single_instance_enabled(true, false, None), "smoke test");
        assert!(
            !single_instance_enabled(true, false, Some("1")),
            "smoke test"
        );
        // Debug builds.
        assert!(!single_instance_enabled(false, true, None));
        assert!(!single_instance_enabled(false, true, Some("0")));
        assert!(single_instance_enabled(false, true, Some("1")));
        assert!(!single_instance_enabled(true, true, Some("1")));
    }
}
