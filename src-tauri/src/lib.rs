//! Desktop shell: wires the engine and the project model to the UI through Tauri.

mod autosave;
mod ffmpeg_check;
mod import;
mod ipc;
mod jobs;
mod launch;
mod logging;
mod media_paths;
mod media_status;
mod preferences;
mod preview;
mod previews;
mod project_file;
mod protocol;
mod recent;
mod recovery;
mod relink;
mod smoke_test;
mod state;
mod updater;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use app_core::Project;
use tauri::{AppHandle, Manager, Window, WindowEvent};
use tauri_plugin_window_state::StateFlags;
use tauri_specta::{Builder, Event, collect_commands, collect_events};

use crate::ipc::commands;
use crate::ipc::events::{
    AppInfoChanged, JobFinished, JobProgress, MediaJobReport, OpenFileRequested, PlaybackTick,
    ProjectChanged, QuitRequested, UpdateProgress,
};
use crate::jobs::{JobManager, Priority, TauriJobHost};
use crate::preview::Preview;
use crate::previews::Previews;
use crate::recent::RecentStore;
use crate::recovery::RecoveryStore;
use crate::state::{AppState, PreferencesState};

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
            commands::add_clips,
            commands::move_clips,
            commands::trim_clip,
            commands::split_clips,
            commands::delete_clips,
            commands::delete_gap,
            commands::trim_to_playhead,
            commands::set_magnetic,
            commands::copy_clips,
            commands::cut_clips,
            commands::paste_clips,
            commands::duplicate_clips,
            commands::link_clips,
            commands::unlink_clips,
            commands::detach_audio,
            commands::replace_clip_media,
            commands::set_clip_property,
            commands::set_clip_speed,
            commands::set_clip_reverse,
            commands::insert_freeze_frame,
            commands::set_track_flag,
            commands::move_track,
            commands::remove_track,
            commands::rename_track,
            commands::rename_project,
            commands::cancel_gesture,
            commands::undo,
            commands::redo,
            commands::jump_to_history,
            commands::get_app_info,
            commands::get_about_info,
            commands::open_logs_folder,
            commands::set_preview_rect,
            commands::set_preview_visible,
            commands::seek,
            commands::toggle_playback,
            commands::pause_playback,
            commands::shuttle,
            commands::step_frames,
            commands::set_loop,
            commands::get_playback,
            commands::new_project,
            commands::open_project,
            commands::save_project,
            commands::save_project_as,
            commands::set_project_settings,
            commands::confirm_close,
            commands::list_recovery,
            commands::restore_recovery,
            commands::discard_recovery,
            commands::import_media,
            commands::import_media_dialog,
            commands::cancel_job,
            commands::remove_media,
            commands::relink_media,
            commands::relink_media_folder,
            commands::reveal_media,
            commands::take_launch_path,
            commands::get_preferences,
            commands::set_preferences,
            commands::get_cache_info,
            commands::clear_cache,
            commands::list_recent_projects,
            commands::remove_recent_project,
            commands::rename_project_file,
            commands::duplicate_project_file,
            commands::trash_project_file,
            commands::reveal_project_file,
            commands::check_for_update,
            commands::install_update,
            commands::restart_app,
        ])
        .events(collect_events![
            ProjectChanged,
            QuitRequested,
            JobProgress,
            JobFinished,
            MediaJobReport,
            OpenFileRequested,
            AppInfoChanged,
            UpdateProgress,
            PlaybackTick
        ])
        // The only place the UI gets the project file extension from (ADR-0002 amendment §1).
        .constant("PROJECT_FILE_EXTENSION", app_core::PROJECT_FILE_EXTENSION)
        // Lowercase, without dots: the drop filter of the media panel (design §4).
        .constant("MEDIA_EXTENSIONS", engine::media::MEDIA_EXTENSIONS)
        .constant("MAX_IMPORT_FILES", import::MAX_IMPORT_FILES)
        // Media panel previews over `media://` (design §4).
        .constant(
            "FILMSTRIP_FRAME_HEIGHT",
            app_core::media::FILMSTRIP_FRAME_HEIGHT,
        )
        .constant(
            "WAVEFORM_BUCKETS_PER_SECOND",
            app_core::media::WAVEFORM_BUCKETS_PER_SECOND,
        )
        .constant("POSTER_LONG_SIDE", app_core::media::POSTER_LONG_SIDE)
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
    let context = tauri::generate_context!();
    // First: the FFmpeg check and early panics already go to the log file.
    let log_dir = logging::init(logging::log_dir(&context.config().identifier));

    // LGPL-only rule: log the linked FFmpeg, refuse a GPL/nonfree build in release.
    ffmpeg_check::check();

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

    let mut builder = tauri::Builder::default();
    // First plugin: a second process hands its arguments over and exits inside plugin init,
    // before `setup`, so it never takes a recovery lock or reads preferences.
    let single_instance_env = std::env::var(launch::SINGLE_INSTANCE_ENV).ok();
    if launch::single_instance_enabled(
        smoke_test,
        cfg!(debug_assertions),
        single_instance_env.as_deref(),
    ) {
        builder = builder.plugin(tauri_plugin_single_instance::init(on_second_instance));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        // Rust side only (`updater.rs`, typed commands); the UI gets no updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Rust side only: restores the main window on creation and saves it on exit. The UI does
        // not call its JS API, so no capability permission is granted.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .invoke_handler(ipc.invoke_handler())
        .on_window_event(on_window_event)
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, protocol::handle)
        .setup(move |app| {
            ipc.mount_events(app);
            check_log_dir(app.handle(), log_dir.as_deref());
            app.manage(logging::LogDir(log_dir));
            app.manage(updater::Updates::default());

            let main = app
                .get_webview_window(MAIN_WINDOW)
                .ok_or("main window missing from tauri.conf.json")?;
            let project = Project::new(commands::DEFAULT_PROJECT_NAME);
            let app_data_dir = app.path().app_data_dir()?;
            let recent = RecentStore::new(&app_data_dir);
            let recovery = RecoveryStore::new(app_data_dir.join("recovery"), uuid::Uuid::now_v7());
            let jobs = JobManager::new(Arc::new(TauriJobHost::new(app.handle().clone())));
            let preferences = load_preferences(app.handle())?;
            let interval = preferences.get().autosave_duration();
            let cwd = std::env::current_dir().unwrap_or_default();
            let launch_path = launch::project_path_from_args(&launch::own_args(), &cwd);
            let previews = Arc::new(Previews::new(crate::previews::open_cache(
                &preferences.cache_root(),
            )));
            let spawner = jobs.spawner();
            // Delete old cache versions and trim the current one, in the background.
            let housekeeping = Arc::clone(&previews);
            spawner.run(Priority::Background, move || housekeeping.clean_up());
            app.manage(protocol::MediaServer::start(protocol::context(
                app.handle(),
                Arc::clone(&previews),
                spawner,
                recent.covers_dir().to_path_buf(),
            )));
            // Runtime offline detection: a preview generator that finds the file gone.
            let handle = app.handle().clone();
            previews.on_missing(Box::new(move |asset| {
                if let Some(state) = handle.try_state::<AppState>() {
                    state.report_missing(&handle, asset);
                }
            }));
            let state = AppState::new(project, recovery.clone(), jobs, previews);
            // The engine reads the state's snapshot; every publish wakes it.
            let preview = Preview::create(&main, state.shared_snapshot());
            app.manage(
                state
                    .with_preview(preview)
                    .with_preferences(preferences)
                    .with_launch_path(launch_path)
                    .with_recent(recent),
            );
            start_autosave(app.handle(), recovery, interval);

            if smoke_test {
                smoke_test::run(app.handle().clone());
            }
            Ok(())
        })
        .run(context)
        .expect("error while running the application");
}

/// The log folder is resolved before Tauri starts; warns if Tauri would put it elsewhere.
fn check_log_dir(app: &AppHandle, log_dir: Option<&Path>) {
    match (app.path().app_log_dir(), log_dir) {
        (Ok(tauri_dir), Some(dir)) if tauri_dir != dir => tracing::warn!(
            log_dir = %dir.display(),
            app_log_dir = %tauri_dir.display(),
            "the log folder differs from Tauri's app_log_dir"
        ),
        _ => {}
    }
}

/// Reads the preferences (defaults when missing or invalid) and creates the cache root. A < 1 KB
/// read on the main thread, once.
fn load_preferences(app: &AppHandle) -> tauri::Result<PreferencesState> {
    let path = app.path().app_config_dir()?.join(preferences::FILE_NAME);
    let startup = preferences::read_startup(&path);
    let app_cache_dir = app.path().app_cache_dir()?;
    let state = PreferencesState::new(startup, path, app_cache_dir);
    let root = state.cache_root();
    if let Err(err) = std::fs::create_dir_all(&root) {
        tracing::warn!(%err, root = %root.display(), "cannot create the cache folder");
    }
    Ok(state)
}

/// A second launch (e.g. a double-clicked project file while the app runs): bring the main window
/// to the front, then ask the UI to open the path it passed, if any.
fn on_second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    let path = launch::project_path_from_args(&argv, Path::new(&cwd));
    tracing::info!(?path, "second instance started");
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        // Windows' foreground lock may refuse focus; then the taskbar button flashes.
        for result in [window.unminimize(), window.show(), window.set_focus()] {
            if let Err(err) = result {
                tracing::warn!(%err, "cannot bring the main window to the front");
            }
        }
    }
    if let Some(path) = path {
        let event = OpenFileRequested {
            path: path.to_string_lossy().into_owned(),
        };
        if let Err(err) = event.emit(app) {
            tracing::error!(%err, "cannot forward the opened file to the UI");
        }
    }
}

/// Takes this session's recovery lock and starts the autosave thread with `interval` (`None` =
/// off). Failures only disable autosave (logged); the app still starts.
fn start_autosave(app: &AppHandle, recovery: RecoveryStore, interval: Option<Duration>) {
    tracing::info!(session = %recovery.session_id(), "recovery session");
    let lock = recovery
        .acquire_lock()
        .inspect_err(|err| tracing::warn!(%err, "cannot lock the recovery directory"))
        .ok();
    let handle = app.clone();
    let snapshot = move || handle.state::<AppState>().save_snapshot();
    match autosave::spawn(recovery, lock, interval, snapshot) {
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

    /// The installer's file association must use the extension the app saves with
    /// (ADR-0002 amendment §1). The JSON cannot reference the constant, so this test ties them.
    #[test]
    fn file_association_uses_the_project_extension() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("tauri.conf.json"))
                .expect("tauri.conf.json is JSON");
        let associations = config["bundle"]["fileAssociations"]
            .as_array()
            .unwrap_or_else(|| panic!("{path}: bundle.fileAssociations is missing"));
        assert_eq!(associations.len(), 1, "one association: {associations:?}");
        assert_eq!(
            associations[0]["ext"],
            serde_json::json!([app_core::PROJECT_FILE_EXTENSION]),
            "bundle.fileAssociations[0].ext must be [PROJECT_FILE_EXTENSION]"
        );
    }
}
