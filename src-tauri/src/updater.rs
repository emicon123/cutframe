//! Automatic updates (F0.12): check, download, then install on restart.
//!
//! The UI only uses the typed commands in `ipc/commands.rs`; the plugin's JS API gets no
//! capability. One update is held between `check_for_update` and `restart_app` ([`UpdateSlot`]).
//! On Windows the NSIS installer replaces the app while it is closed: `install` starts it in
//! `passive` mode with `/R` (start the app again afterwards) and exits this process.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};
use tauri_specta::Event;

use crate::ipc::IpcError;
use crate::ipc::events::UpdateProgress;
use crate::ipc::types::UpdateInfo;
use crate::state::AppState;

/// Debug builds check for updates only with `CUTFRAME_UPDATER=1`: `pnpm dev` runs a version that
/// is not an installed release and must not offer to replace the installed app.
pub const UPDATER_ENV: &str = "CUTFRAME_UPDATER";

/// The `plugins.updater.pubkey` placeholder in `tauri.conf.json` until the real key exists
/// (`pnpm tauri signer generate`). It is not a valid key, so checks report `unavailable`.
pub const PUBKEY_PLACEHOLDER: &str =
    "PLACEHOLDER: paste the public key printed by `pnpm tauri signer generate` here";

/// The update server must answer within these; there is no limit on the whole download, which
/// may take minutes on a slow line.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// At most 10 `downloading` events per second (like `jobProgress`).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Why checks are off, or `Ok` when they may run: always in release builds, in debug builds only
/// with [`UPDATER_ENV`]` = 1`, and never without a valid public key (the placeholder is not one).
pub fn availability(
    debug_build: bool,
    env: Option<&str>,
    pubkey: Option<&str>,
) -> Result<(), IpcError> {
    if debug_build && env != Some("1") {
        return Err(IpcError::Unavailable(format!(
            "update checks are off in debug builds; set {UPDATER_ENV}=1 to turn them on"
        )));
    }
    match pubkey {
        Some(PUBKEY_PLACEHOLDER) => Err(IpcError::Unavailable(
            "the updater public key is not set yet (plugins.updater.pubkey is the placeholder)"
                .into(),
        )),
        Some(key) if pubkey_is_valid(key) => Ok(()),
        _ => Err(IpcError::Unavailable(
            "no valid updater public key in tauri.conf.json (plugins.updater.pubkey)".into(),
        )),
    }
}

/// Whether `pubkey` decodes like the plugin decodes it: base64 of a minisign public key file
/// (comment line + key line), as `tauri signer generate` prints it.
pub fn pubkey_is_valid(pubkey: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(pubkey)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|text| minisign_verify::PublicKey::decode(&text).is_ok())
}

/// Maps an updater failure to the kinds the UI translates: `unavailable` (no server, no release,
/// bad `latest.json`), `updateRejected` (signature or file did not verify), `io`, else `internal`.
pub fn to_ipc_error(err: UpdaterError) -> IpcError {
    let message = err.to_string();
    match err {
        UpdaterError::Reqwest(_)
        | UpdaterError::Network(_)
        | UpdaterError::ReleaseNotFound
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_)
        | UpdaterError::EmptyEndpoints
        | UpdaterError::InsecureTransportProtocol
        | UpdaterError::UnsupportedArch
        | UpdaterError::UnsupportedOs
        | UpdaterError::Serialization(_)
        | UpdaterError::Semver(_)
        | UpdaterError::UrlParse(_) => IpcError::Unavailable(message),
        UpdaterError::Minisign(_)
        | UpdaterError::Base64(_)
        | UpdaterError::SignatureUtf8(_)
        | UpdaterError::SignedVersionMismatch { .. }
        | UpdaterError::MissingSignedVersion
        | UpdaterError::InvalidUpdaterFormat
        | UpdaterError::BinaryNotFoundInArchive => IpcError::UpdateRejected(message),
        UpdaterError::Io(_) => IpcError::Io(message),
        _ => IpcError::Internal(message),
    }
}

/// The one update between check and install. Generic so the transitions are testable without
/// a server.
#[derive(Debug, Default)]
pub enum UpdateSlot<U> {
    #[default]
    Idle,
    /// Found by a check; `install_update` may start.
    Available(U),
    Downloading(U),
    /// Downloaded and verified; `restart_app` installs it.
    Ready {
        update: U,
        bytes: Vec<u8>,
    },
    /// The installer is being started; the process exits next.
    Installing(U),
}

/// What `install_update` has to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Start<U> {
    Download(U),
    /// Already downloading: one install at a time, the running download keeps reporting.
    AlreadyRunning,
    /// Already downloaded: report `ready` again.
    AlreadyReady,
}

impl<U: Clone> UpdateSlot<U> {
    /// The update a download or install is busy with; a check then answers with it.
    pub fn busy(&self) -> Option<&U> {
        match self {
            Self::Downloading(u) | Self::Ready { update: u, .. } | Self::Installing(u) => Some(u),
            Self::Idle | Self::Available(_) => None,
        }
    }

    /// A check's result. Ignored while busy, so a late check never replaces the download.
    pub fn offer(&mut self, found: Option<U>) {
        if self.busy().is_none() {
            *self = found.map_or(Self::Idle, Self::Available);
        }
    }

    pub fn start_download(&mut self) -> Result<Start<U>, IpcError> {
        match self {
            Self::Available(update) => {
                let update = update.clone();
                *self = Self::Downloading(update.clone());
                Ok(Start::Download(update))
            }
            Self::Downloading(_) => Ok(Start::AlreadyRunning),
            Self::Ready { .. } => Ok(Start::AlreadyReady),
            Self::Idle => Err(IpcError::Internal(
                "no update to install; call check_for_update first".into(),
            )),
            Self::Installing(_) => Err(IpcError::Internal(
                "the update is already being installed".into(),
            )),
        }
    }

    /// The download's end: `Some(bytes)` makes it ready, `None` (failed) lets the user retry.
    pub fn download_finished(&mut self, bytes: Option<Vec<u8>>) {
        if let Self::Downloading(update) = self {
            let update = update.clone();
            *self = match bytes {
                Some(bytes) => Self::Ready { update, bytes },
                None => Self::Available(update),
            };
        }
    }

    /// Takes a ready update to install it; `None` when nothing is ready.
    pub fn take_ready(&mut self) -> Option<(U, Vec<u8>)> {
        let Self::Ready { update, .. } = self else {
            return None;
        };
        let installing = Self::Installing(update.clone());
        match std::mem::replace(self, installing) {
            Self::Ready { update, bytes } => Some((update, bytes)),
            _ => unreachable!("checked above"),
        }
    }

    /// Starting the installer failed: the update stays ready for another try.
    pub fn install_failed(&mut self, update: U, bytes: Vec<u8>) {
        *self = Self::Ready { update, bytes };
    }
}

/// Turns download chunks into at most one `downloading` event per [`PROGRESS_INTERVAL`].
#[derive(Debug, Default)]
pub struct ProgressThrottle {
    downloaded: u64,
    last: Option<Instant>,
}

impl ProgressThrottle {
    pub fn chunk(
        &mut self,
        len: usize,
        total: Option<u64>,
        now: Instant,
    ) -> Option<UpdateProgress> {
        self.downloaded += len as u64;
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < PROGRESS_INTERVAL)
        {
            return None;
        }
        self.last = Some(now);
        Some(UpdateProgress::Downloading {
            downloaded: self.downloaded.to_string(),
            total: total.map(|total| total.to_string()),
        })
    }
}

/// App state: the held update.
#[derive(Default)]
pub struct Updates(Mutex<UpdateSlot<Update>>);

impl Updates {
    fn lock(&self) -> MutexGuard<'_, UpdateSlot<Update>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn info(update: &Update) -> UpdateInfo {
    // `date` parsed from `pub_date`; send the release's own RFC 3339 text.
    let date = update
        .date
        .and(update.raw_json.get("pub_date"))
        .and_then(|date| date.as_str())
        .map(str::to_owned);
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        date,
        notes: update.body.clone(),
    }
}

fn configured_pubkey(app: &AppHandle) -> Option<String> {
    let config = app.config().plugins.0.get("updater")?;
    Some(config.get("pubkey")?.as_str()?.to_owned())
}

fn emit(app: &AppHandle, event: UpdateProgress) {
    if let Err(err) = event.emit(app) {
        tracing::error!(%err, "cannot send update progress to the UI");
    }
}

/// `check_for_update`: asks the endpoint for a newer release.
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, IpcError> {
    let env = std::env::var(UPDATER_ENV).ok();
    let pubkey = configured_pubkey(app);
    availability(cfg!(debug_assertions), env.as_deref(), pubkey.as_deref())?;
    let updates = app.state::<Updates>();
    if let Some(update) = updates.lock().busy() {
        return Ok(Some(info(update)));
    }
    let handle = app.clone();
    let updater = app
        .updater_builder()
        .configure_client(|client| {
            client
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
        })
        // Windows: right before the installer starts and the process exits.
        .on_before_exit(move || {
            if let Some(state) = handle.try_state::<AppState>() {
                state.shutdown_autosave();
            }
            handle.cleanup_before_exit();
        })
        .build()
        .map_err(to_ipc_error)?;
    let found = updater.check().await.map_err(to_ipc_error)?;
    let result = found.as_ref().map(info);
    match &result {
        Some(info) => tracing::info!(version = %info.version, "update available"),
        None => tracing::info!("no update available"),
    }
    updates.lock().offer(found);
    Ok(result)
}

/// `install_update`: starts downloading the update found by the last check, at most once.
pub fn install(app: &AppHandle) -> Result<(), IpcError> {
    let start = app.state::<Updates>().lock().start_download()?;
    match start {
        Start::Download(update) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { download(app, update).await });
        }
        Start::AlreadyRunning => {}
        Start::AlreadyReady => emit(app, UpdateProgress::Ready),
    }
    Ok(())
}

async fn download(app: AppHandle, update: Update) {
    tracing::info!(version = %update.version, "downloading the update");
    let mut throttle = ProgressThrottle::default();
    let result = update
        .download(
            |len, total| {
                if let Some(event) = throttle.chunk(len, total, Instant::now()) {
                    emit(&app, event);
                }
            },
            || {},
        )
        .await;
    let updates = app.state::<Updates>();
    match result {
        Ok(bytes) => {
            tracing::info!(bytes = bytes.len(), "update downloaded and verified");
            updates.lock().download_finished(Some(bytes));
            emit(&app, UpdateProgress::Ready);
        }
        Err(err) => {
            let error = to_ipc_error(err);
            tracing::warn!(?error, "update download failed");
            updates.lock().download_finished(None);
            emit(&app, UpdateProgress::Failed { error });
        }
    }
}

/// `restart_app`: installs a ready update (which restarts the app), else restarts the app.
pub async fn restart(app: AppHandle) -> Result<(), IpcError> {
    let ready = app.state::<Updates>().lock().take_ready();
    if let Some((update, bytes)) = ready {
        return install_ready(&app, update, bytes).await;
    }
    tracing::info!("restarting the app");
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || handle.state::<AppState>().shutdown_autosave())
        .await?;
    app.request_restart();
    Ok(())
}

/// Verifies again, writes the installer to a temp folder, starts it and exits: returns only when
/// that failed.
#[cfg(windows)]
async fn install_ready(app: &AppHandle, update: Update, bytes: Vec<u8>) -> Result<(), IpcError> {
    tracing::info!(version = %update.version, "installing the update");
    let (update, bytes, result) = tauri::async_runtime::spawn_blocking(move || {
        let result = update.install(&bytes);
        (update, bytes, result)
    })
    .await?;
    let error = match result {
        Err(err) => to_ipc_error(err),
        Ok(()) => IpcError::Internal("the installer started but the app did not exit".into()),
    };
    tracing::error!(?error, "cannot install the update");
    app.state::<Updates>().lock().install_failed(update, bytes);
    Err(error)
}

/// Only the Windows NSIS build installs updates.
#[cfg(not(windows))]
async fn install_ready(app: &AppHandle, update: Update, bytes: Vec<u8>) -> Result<(), IpcError> {
    app.state::<Updates>().lock().install_failed(update, bytes);
    Err(IpcError::Unavailable(
        "installing updates is only supported on Windows".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syntactically valid minisign public key (not one anybody holds the secret key of), in
    /// the form `tauri signer generate` prints.
    fn sample_pubkey() -> String {
        let engine = base64::engine::general_purpose::STANDARD;
        let mut key = b"Ed".to_vec();
        key.extend([7; 8]);
        key.extend([9; 32]);
        let file = format!(
            "untrusted comment: minisign public key: 0707070707070707\n{}\n",
            engine.encode(key)
        );
        engine.encode(file)
    }

    fn kind(error: IpcError) -> String {
        serde_json::to_value(error).unwrap()["kind"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn debug_builds_check_only_with_the_env_opt_in() {
        let key = sample_pubkey();
        let key = Some(key.as_str());
        assert!(availability(false, None, key).is_ok());
        assert!(availability(true, Some("1"), key).is_ok());
        for env in [None, Some("0"), Some("true"), Some("")] {
            assert_eq!(
                kind(availability(true, env, key).unwrap_err()),
                "unavailable"
            );
        }
    }

    #[test]
    fn the_placeholder_or_a_bad_key_turns_checks_off() {
        assert!(pubkey_is_valid(&sample_pubkey()));
        for key in [
            None,
            Some(PUBKEY_PLACEHOLDER),
            Some(""),
            Some("bm90IGEga2V5"), // base64 of "not a key"
        ] {
            assert!(!key.is_some_and(pubkey_is_valid), "{key:?}");
            assert_eq!(
                kind(availability(false, None, key).unwrap_err()),
                "unavailable"
            );
        }
    }

    /// `tauri.conf.json` must parse as the plugin's config (or the app does not start) and keep
    /// the per-user passive install and the signed-version check; the key is either the
    /// placeholder or a valid key.
    #[test]
    fn updater_config_is_valid() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("tauri.conf.json"))
                .expect("tauri.conf.json is JSON");
        let updater = &config["plugins"]["updater"];
        let parsed: tauri_plugin_updater::Config =
            serde_json::from_value(updater.clone()).expect("plugins.updater parses");
        assert!(!parsed.endpoints.is_empty());
        assert!(parsed.endpoints.iter().all(|url| url.scheme() == "https"));
        // No release predates this: every signature carries the version (`tauri build` passes
        // `--app-version`), so a tampered `latest.json` cannot pair a new version with an old file.
        assert!(parsed.require_signed_version);
        assert!(!parsed.allow_downgrades);
        assert_eq!(updater["windows"]["installMode"], "passive");
        assert!(
            parsed.pubkey == PUBKEY_PLACEHOLDER || pubkey_is_valid(&parsed.pubkey),
            "plugins.updater.pubkey is neither the placeholder nor a valid key"
        );
        assert_eq!(
            config["bundle"]["windows"]["nsis"]["installMode"],
            "currentUser"
        );
    }

    #[test]
    fn updater_errors_map_to_their_kinds() {
        let cases = [
            (UpdaterError::ReleaseNotFound, "unavailable"),
            (UpdaterError::Network("503".into()), "unavailable"),
            (
                UpdaterError::TargetNotFound("windows-x86_64".into()),
                "unavailable",
            ),
            (UpdaterError::EmptyEndpoints, "unavailable"),
            (
                UpdaterError::Serialization(serde_json::from_str::<u8>("x").unwrap_err()),
                "unavailable",
            ),
            (
                UpdaterError::Minisign(minisign_verify::Error::InvalidEncoding),
                "updateRejected",
            ),
            (
                UpdaterError::SignedVersionMismatch {
                    signed: "0.1.0".into(),
                    announced: "0.2.0".into(),
                },
                "updateRejected",
            ),
            (UpdaterError::InvalidUpdaterFormat, "updateRejected"),
            (
                UpdaterError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                "io",
            ),
            (UpdaterError::TempDirNotFound, "internal"),
        ];
        for (error, expected) in cases {
            assert_eq!(kind(to_ipc_error(error)), expected);
        }
    }

    #[test]
    fn a_found_update_downloads_once_then_installs() {
        let mut slot = UpdateSlot::default();
        assert_eq!(kind(slot.start_download().unwrap_err()), "internal");
        slot.offer(Some("0.2.0"));
        assert_eq!(slot.busy(), None);
        assert_eq!(slot.start_download(), Ok(Start::Download("0.2.0")));
        // One install at a time.
        assert_eq!(slot.start_download(), Ok(Start::AlreadyRunning));
        assert_eq!(slot.busy(), Some(&"0.2.0"));
        slot.download_finished(Some(vec![1, 2, 3]));
        assert_eq!(slot.start_download(), Ok(Start::AlreadyReady));
        assert_eq!(slot.take_ready(), Some(("0.2.0", vec![1, 2, 3])));
        assert_eq!(slot.take_ready(), None);
        assert_eq!(kind(slot.start_download().unwrap_err()), "internal");
        slot.install_failed("0.2.0", vec![1, 2, 3]);
        assert_eq!(slot.take_ready(), Some(("0.2.0", vec![1, 2, 3])));
    }

    #[test]
    fn a_failed_download_can_be_retried() {
        let mut slot = UpdateSlot::default();
        slot.offer(Some(1));
        assert_eq!(slot.start_download(), Ok(Start::Download(1)));
        slot.download_finished(None);
        assert_eq!(slot.busy(), None);
        assert_eq!(slot.take_ready(), None);
        assert_eq!(slot.start_download(), Ok(Start::Download(1)));
    }

    #[test]
    fn a_check_never_replaces_a_running_download() {
        let mut slot = UpdateSlot::default();
        slot.offer(Some(1));
        slot.offer(Some(2));
        assert_eq!(slot.start_download(), Ok(Start::Download(2)));
        slot.offer(Some(3));
        slot.offer(None);
        assert_eq!(slot.busy(), Some(&2));
        slot.download_finished(Some(vec![]));
        slot.offer(None);
        assert_eq!(slot.take_ready(), Some((2, vec![])));
    }

    #[test]
    fn a_check_without_update_clears_an_older_offer() {
        let mut slot = UpdateSlot::default();
        slot.offer(Some(1));
        slot.offer(None);
        assert_eq!(kind(slot.start_download().unwrap_err()), "internal");
    }

    #[test]
    fn progress_is_throttled_and_counts_every_chunk() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::default();
        let downloading = |downloaded: &str, total: Option<&str>| UpdateProgress::Downloading {
            downloaded: downloaded.into(),
            total: total.map(Into::into),
        };
        assert_eq!(
            throttle.chunk(10, Some(100), start),
            Some(downloading("10", Some("100")))
        );
        assert_eq!(
            throttle.chunk(10, Some(100), start + PROGRESS_INTERVAL / 2),
            None
        );
        assert_eq!(
            throttle.chunk(5, None, start + PROGRESS_INTERVAL),
            Some(downloading("25", None))
        );
    }

    #[test]
    fn progress_serializes_with_a_state_tag() {
        let json = |event: UpdateProgress| serde_json::to_value(event).unwrap();
        assert_eq!(
            json(UpdateProgress::Downloading {
                downloaded: "5".into(),
                total: None
            }),
            serde_json::json!({ "state": "downloading", "downloaded": "5", "total": null })
        );
        assert_eq!(
            json(UpdateProgress::Ready),
            serde_json::json!({ "state": "ready" })
        );
        assert_eq!(
            json(UpdateProgress::Failed {
                error: IpcError::UpdateRejected("x".into())
            }),
            serde_json::json!({
                "state": "failed",
                "error": { "kind": "updateRejected", "message": "x" }
            })
        );
    }
}
