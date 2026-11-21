fn main() {
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
        println!("cargo:rustc-link-arg=/WX");
    }

    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
