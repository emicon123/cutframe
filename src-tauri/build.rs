fn main() {
    emit_commit();

    // tauri-build embeds the Common Controls v6 manifest only into binaries, so on Windows the
    // `cargo test` executables fail to start with STATUS_ENTRYPOINT_NOT_FOUND (comctl32
    // `TaskDialogIndirect` exists only in v6). Embed the manifest through the MSVC linker for every
    // target instead, tests included.
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let mut attributes = tauri_build::Attributes::new();
    if windows_msvc {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());

        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        // Fail the link instead of silently producing a binary without the manifest.
        //
        // Kept after FFmpeg started linking (review F43): the FFmpeg import libraries
        // (`.ffmpeg\lib\*.lib`, linked through rusty_ffmpeg with FFMPEG_LINK_MODE=dynamic) are
        // import-only, with no object code or debug info, so they do not raise the LNK4099
        // ("PDB not found") warnings static libraries do. If a specific FFmpeg-related linker
        // warning ever appears, silence that one code with `/IGNORE:<code>` here, with a comment,
        // instead of dropping `/WX`.
        println!("cargo:rustc-link-arg=/WX");
    }

    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

/// Sets `CUTFRAME_COMMIT` (short commit hash, for the About dialog) from `GITHUB_SHA` in CI or from
/// `git` locally. Left unset when neither is available (e.g. a source archive).
fn emit_commit() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    let commit = std::env::var("GITHUB_SHA")
        .ok()
        .filter(|sha| !sha.is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]));
    // Every commit, checkout or reset moves HEAD and appends to its reflog.
    if let Some(reflog) = git(&["rev-parse", "--git-path", "logs/HEAD"])
        && std::path::Path::new(&reflog).exists()
    {
        println!("cargo:rerun-if-changed={reflog}");
    }
    if let Some(commit) = commit {
        let short: String = commit.chars().take(12).collect();
        println!("cargo:rustc-env=CUTFRAME_COMMIT={short}");
    }
}

/// Trimmed stdout of a successful `git` call, run in this crate's folder.
fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_owned())
}
