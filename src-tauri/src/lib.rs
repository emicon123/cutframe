//! Desktop shell: wires the engine and the project model to the UI through Tauri.

mod autosave;
mod ipc;
mod preview;
mod project_file;
mod protocol;
mod recovery;
mod smoke_test;
mod state;

use app_core::Project;
use tauri::{Manager, Window, WindowEvent};
use tauri_specta::{Builder, Event, collect_commands, collect_events};
use tracing_subscriber::EnvFilter;

use crate::ipc::commands;
use crate::ipc::events::{ProjectChanged, QuitRequested};
use crate::preview::Preview;
use crate::recovery::RecoveryStore;
use crate::state::AppState;

/// Where the generated TypeScript bindings live. Committed to the repo.
pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");

/// Label of the editor window in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// The full IPC surface. Adding a command or event here is all it takes to expose it to the UI.
pub fn ipc_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::get_project,
            commands::add_track,
            commands::remove_track,
            commands::rename_track,
            commands::rename_project,
            commands::cancel_gesture,
            commands::undo,
            commands::redo,
            commands::get_app_info,
            commands::set_preview_rect,
            commands::set_preview_visible,
            commands::new_project,
            commands::open_project,
            commands::save_project,
            commands::save_project_as,
            commands::set_project_settings,
            commands::confirm_close,
            commands::list_recovery,
            commands::restore_recovery,
            commands::discard_recovery,
        ])
        .events(collect_events![ProjectChanged, QuitRequested])
        // The only place the UI gets the project file extension from (ADR-0002 amendment §1).
        .constant("PROJECT_FILE_EXTENSION", app_core::PROJECT_FILE_EXTENSION)
        // The phase argument of gesture-capable commands (ADR-0006 §4). Exported on its own so the
        // UI's gesture helper can use it before the first such command lands.
        .typ::<app_core::EditPhase>()
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

    // `--smoke-test`: render offscreen and (Windows) into the preview window, then exit with a
    // status code; see `smoke_test.rs` for the codes. Used by CI against the installed build.
    let smoke_test = std::env::args().any(|arg| arg == smoke_test::FLAG);
    if smoke_test {
        smoke_test::start_watchdog();
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(ipc.invoke_handler())
        .on_window_event(on_window_event)
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .setup(move |app| {
            ipc.mount_events(app);

            let main = app
                .get_webview_window(MAIN_WINDOW)
                .ok_or("main window missing from tauri.conf.json")?;
            let project = Project::new(commands::DEFAULT_PROJECT_NAME);
            let preview = Preview::create(&main, &project.settings);
            let recovery = RecoveryStore::new(
                app.path().app_data_dir()?.join("recovery"),
                uuid::Uuid::now_v7(),
            );
            app.manage(AppState::new(project, preview, recovery.clone()));
            start_autosave(app.handle(), recovery);

            if smoke_test {
                smoke_test::run(app.handle().clone());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the application");
}

/// Takes this session's recovery lock and starts the autosave thread. Failures only disable
/// autosave (logged); the app still starts.
fn start_autosave(app: &tauri::AppHandle, recovery: RecoveryStore) {
    tracing::info!(session = %recovery.session_id(), "recovery session");
    let lock = recovery
        .acquire_lock()
        .inspect_err(|err| tracing::warn!(%err, "cannot lock the recovery directory"))
        .ok();
    let handle = app.clone();
    let snapshot = move || handle.state::<AppState>().save_snapshot();
    match autosave::spawn(recovery, lock, snapshot) {
        Ok(autosave) => app.state::<AppState>().set_autosave(autosave),
        Err(err) => tracing::error!(%err, "failed to start the autosave thread"),
    }
}

/// Closing the main window with unsaved changes is stopped and handed to the UI, which asks
/// Save / Don't save / Cancel and calls `confirm_close`. A clean project closes right away, even if
/// the webview does not respond.
///
/// When the main window is gone (clean close, or `confirm_close` after Save / Don't save), the
/// autosave thread deletes this session's recovery files.
fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    if let WindowEvent::Destroyed = event {
        if let Some(state) = window.try_state::<AppState>() {
            state.shutdown_autosave();
        }
        return;
    }
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    let Some(state) = window.try_state::<AppState>() else {
        return;
    };
    if !state.is_dirty() {
        return;
    }
    match QuitRequested.emit(window.app_handle()) {
        Ok(()) => api.prevent_close(),
        // Without the event nobody would ever close the window; let it close.
        Err(err) => tracing::error!(%err, "failed to ask the UI about unsaved changes"),
    }
}

#[cfg(test)]
mod tests {
    /// Regenerates the bindings. CI runs this and fails if `git diff` shows changes.
    #[test]
    fn export_typescript_bindings() {
        super::export_bindings().expect("bindings export");
    }
}
