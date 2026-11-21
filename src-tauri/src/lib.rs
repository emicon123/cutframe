//! Desktop shell: wires the engine and the project model to the UI through Tauri.

mod ipc;
mod preview;
mod protocol;
mod state;

use std::time::Duration;

use app_core::Project;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};
use tracing_subscriber::EnvFilter;

use crate::ipc::commands;
use crate::ipc::events::ProjectChanged;
use crate::preview::Preview;
use crate::state::AppState;

/// Where the generated TypeScript bindings live. Committed to the repo.
pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");

/// Command-line flag: start, render one preview frame, exit 0 on success. Used by CI against the
/// installed build.
const SMOKE_TEST_FLAG: &str = "--smoke-test";

/// The full IPC surface. Adding a command or event here is all it takes to expose it to the UI.
pub fn ipc_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::get_project,
            commands::add_track,
            commands::remove_track,
            commands::rename_track,
            commands::rename_project,
            commands::undo,
            commands::redo,
            commands::get_app_info,
            commands::set_preview_rect,
            commands::set_preview_visible,
        ])
        .events(collect_events![ProjectChanged])
        // Time (flicks) and revisions are 64-bit; both stay far below 2^53 in practice.
        .dangerously_cast_bigints_to_number()
        .mocks()
}

/// Writes `ui/src/bindings.ts`.
pub fn export_bindings() -> Result<(), specta_typescript::Error> {
    ipc_builder().export(specta_typescript::Typescript::default(), BINDINGS_PATH)
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")),
        )
        .init();

    let ipc = ipc_builder();
    #[cfg(debug_assertions)]
    if let Err(err) = ipc.export(specta_typescript::Typescript::default(), BINDINGS_PATH) {
        tracing::error!(%err, "failed to export TypeScript bindings");
    }

    let smoke_test = std::env::args().any(|arg| arg == SMOKE_TEST_FLAG);

    tauri::Builder::default()
        .invoke_handler(ipc.invoke_handler())
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .setup(move |app| {
            ipc.mount_events(app);

            let main = app
                .get_webview_window("main")
                .ok_or("main window missing from tauri.conf.json")?;
            let project = Project::new("Untitled project");
            let preview = Preview::create(&main, &project.settings);
            app.manage(AppState::new(project, preview));

            if smoke_test {
                run_smoke_test(app.handle().clone());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the application");
}

fn run_smoke_test(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let code = match &state.preview {
            Some(preview) => match preview.render_frame(320, 180) {
                Some(frame) if frame.rgba.len() == 320 * 180 * 4 => {
                    let gpu = preview.gpu_info();
                    tracing::info!(adapter = %gpu.adapter, backend = %gpu.backend, "smoke test passed");
                    0
                }
                _ => {
                    tracing::error!("smoke test: no frame rendered");
                    1
                }
            },
            None => {
                tracing::error!("smoke test: no GPU");
                2
            }
        };
        // Give the log a moment to flush before exiting.
        std::thread::sleep(Duration::from_millis(200));
        app.exit(code);
    });
}

#[cfg(test)]
mod tests {
    /// Regenerates the bindings. CI runs this and fails if `git diff` shows changes.
    #[test]
    fn export_typescript_bindings() {
        super::export_bindings().expect("bindings export");
    }
}
