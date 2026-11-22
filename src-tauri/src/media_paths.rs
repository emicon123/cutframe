//! Media file paths relative to the project file (ADR-0002 §7, design `media-import.md` §6.6).
//!
//! - **Save** writes `path.relative` next to `path.absolute`: the media file relative to the
//!   project file's folder, or `None` when there is no such path (another drive).
//! - **Open** (and restore) tries `<project folder>\relative` first, then `absolute`. When the
//!   relative path wins, `absolute` is updated in the loaded project before the `Document` is
//!   created: load-time normalisation, not an edit. Whatever is missing after that is reported
//!   `offline` by the media check job.
//!
//! Both are plain path logic plus `is_file` checks; callers run them off the main thread.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use app_core::Project;

/// Paths as stored and shown: absolute, without the Windows `\\?\` prefix.
pub fn normalise(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let path = match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_owned(),
    };
    std::path::absolute(&path).unwrap_or(path)
}

/// The key that makes two spellings of one path the same file (Windows paths ignore case).
pub fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

fn same_component(a: &Component<'_>, b: &Component<'_>) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// `target` relative to the folder `base`, e.g. `..\Videos\a.mp4`. `None` when either is not
/// absolute, they are on different drives (or shares), or `target` is `base` itself.
pub fn relative_to(base: &Path, target: &Path) -> Option<PathBuf> {
    if !base.is_absolute() || !target.is_absolute() {
        return None;
    }
    let (base_path, target_path) = (lexical(base), lexical(target));
    let base: Vec<Component<'_>> = base_path.components().collect();
    let target: Vec<Component<'_>> = target_path.components().collect();
    // The prefix (drive or share) and the root must match, otherwise there is no relative path.
    let roots = |c: &[Component<'_>]| {
        c.iter()
            .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
            .count()
    };
    let (base_roots, target_roots) = (roots(&base), roots(&target));
    if base_roots != target_roots
        || !base[..base_roots]
            .iter()
            .zip(&target[..target_roots])
            .all(|(a, b)| same_component(a, b))
    {
        return None;
    }
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(a, b)| same_component(a, b))
        .count();
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for part in &target[common..] {
        relative.push(part.as_os_str());
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

/// `path` with `.` and `..` resolved without touching the file system.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `project` as it is written to `file`: every media path gets its `relative` form (or `None`).
/// Borrows when nothing changes. Without a file the project is written as it is.
pub fn with_relative_paths<'a>(project: &'a Project, file: Option<&Path>) -> Cow<'a, Project> {
    let Some(dir) = file.and_then(Path::parent) else {
        return Cow::Borrowed(project);
    };
    let relative = |absolute: &str| {
        relative_to(dir, Path::new(absolute)).map(|p| p.to_string_lossy().into_owned())
    };
    if project
        .media
        .iter()
        .all(|m| m.path.relative == relative(&m.path.absolute))
    {
        return Cow::Borrowed(project);
    }
    let mut written = project.clone();
    for asset in &mut written.media {
        asset.path.relative = relative(&asset.path.absolute);
    }
    Cow::Owned(written)
}

/// Load-time resolution for a project read from (or last saved to) `file`: an asset whose
/// relative path names an existing file there gets that file as its `absolute` path; otherwise
/// `absolute` stays as stored. Returns how many paths changed.
pub fn resolve_paths(project: &mut Project, file: &Path) -> usize {
    let Some(dir) = file.parent() else {
        return 0;
    };
    let mut changed = 0;
    for asset in &mut project.media {
        let Some(relative) = &asset.path.relative else {
            continue;
        };
        let candidate = lexical(&dir.join(relative));
        if !candidate.is_absolute() || !candidate.is_file() {
            continue;
        }
        let Some(found) = candidate.to_str() else {
            continue;
        };
        if path_key(Path::new(found)) != path_key(Path::new(&asset.path.absolute)) {
            tracing::info!(media = %asset.id, from = %asset.path.absolute, to = found, "media found by its relative path");
            asset.path.absolute = found.to_owned();
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use app_core::media::{MediaPath, samples};

    use super::*;

    fn abs(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    fn rel(path: &str) -> PathBuf {
        path.split('/').collect()
    }

    #[test]
    fn relative_paths_climb_and_descend() {
        let base = abs("/Users/me/Projects/trip");
        assert_eq!(
            relative_to(&base, &abs("/Users/me/Projects/trip/media/a.mp4")),
            Some(rel("media/a.mp4"))
        );
        assert_eq!(
            relative_to(&base, &abs("/Users/me/Videos/b.mov")),
            Some(rel("../../Videos/b.mov"))
        );
        assert_eq!(
            relative_to(&base, &abs("/Users/me/Projects/trip/../trip/c.wav")),
            Some(rel("c.wav")),
            "dots are resolved"
        );
        assert_eq!(relative_to(&base, &base), None, "the folder itself");
        assert_eq!(relative_to(&base, Path::new("a.mp4")), None, "not absolute");
    }

    #[cfg(windows)]
    #[test]
    fn no_relative_path_across_drives_and_case_is_ignored() {
        let base = Path::new(r"C:\Projects\trip");
        assert_eq!(relative_to(base, Path::new(r"D:\Videos\a.mp4")), None);
        assert_eq!(
            relative_to(base, Path::new(r"c:\projects\TRIP\media\a.mp4")),
            Some(PathBuf::from(r"media\a.mp4"))
        );
        assert_eq!(
            relative_to(base, Path::new(r"\\nas\share\a.mp4")),
            None,
            "a share is another root"
        );
    }

    #[test]
    fn saving_writes_relative_paths_and_borrows_when_current() {
        let dir = abs("/p/trip");
        let file = dir.join("trip.cutframe");
        let mut project = Project::new("Trip");
        let mut video = samples::video(1);
        video.path = MediaPath::absolute(dir.join("media").join("a.mp4").to_string_lossy());
        let mut audio = samples::audio(2);
        audio.path = MediaPath::absolute("relative/not-absolute.wav");
        project.media = vec![video, audio];

        assert!(matches!(
            with_relative_paths(&project, None),
            Cow::Borrowed(_)
        ));
        let written = with_relative_paths(&project, Some(&file)).into_owned();
        assert_eq!(
            written.media[0].path.relative.as_deref(),
            Some(rel("media/a.mp4").to_str().unwrap())
        );
        assert_eq!(written.media[1].path.relative, None);
        assert_eq!(
            written.media[0].path.absolute,
            project.media[0].path.absolute
        );
        assert!(matches!(
            with_relative_paths(&written, Some(&file)),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn opening_prefers_the_relative_path_then_the_absolute_one() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let new = root.path().join("new");
        std::fs::create_dir_all(old.join("media")).unwrap();
        std::fs::write(old.join("media").join("a.mp4"), "a").unwrap();
        std::fs::write(old.join("media").join("b.mp4"), "b").unwrap();
        let outside = root.path().join("outside.wav");
        std::fs::write(&outside, "c").unwrap();

        let mut project = Project::new("Trip");
        for (seed, path) in [
            (1, old.join("media").join("a.mp4")),
            (2, old.join("media").join("b.mp4")),
            (3, outside.clone()),
        ] {
            let mut asset = samples::video(seed);
            asset.path = MediaPath::absolute(path.to_string_lossy());
            project.media.push(asset);
        }
        let saved = with_relative_paths(&project, Some(&old.join("trip.cutframe"))).into_owned();

        // The project folder moves with its media; `b.mp4` is lost on the way.
        std::fs::rename(&old, &new).unwrap();
        std::fs::remove_file(new.join("media").join("b.mp4")).unwrap();
        let mut opened = saved.clone();
        assert_eq!(resolve_paths(&mut opened, &new.join("trip.cutframe")), 1);
        assert_eq!(
            Path::new(&opened.media[0].path.absolute),
            new.join("media").join("a.mp4")
        );
        assert_eq!(
            opened.media[1].path, saved.media[1].path,
            "missing everywhere: stays (offline)"
        );
        assert_eq!(
            opened.media[2].path.absolute, saved.media[2].path.absolute,
            "the relative path is gone, the absolute one still works"
        );
        assert_eq!(
            resolve_paths(&mut opened, &new.join("trip.cutframe")),
            0,
            "idempotent"
        );
    }

    #[test]
    fn windows_long_path_prefix_is_removed() {
        let path = normalise(Path::new(r"\\?\C:\Videos\a.mp4"));
        let text = path.to_string_lossy();
        assert!(!text.contains(r"\\?\"), "{text}");
        assert!(text.ends_with(r"C:\Videos\a.mp4"), "{text}");
        #[cfg(windows)]
        assert_eq!(text, r"C:\Videos\a.mp4");
    }
}
