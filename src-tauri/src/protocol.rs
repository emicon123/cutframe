//! The `media://` protocol: binary assets for the UI that must not go through JSON IPC
//! (ADR-0003 §9, design §4 and §6.3, review F12).
//!
//! Requests go into a bounded queue ([`QUEUE_CAPACITY`], full → `503`) served by
//! [`HANDLER_THREADS`] threads. A handler parses the route, resolves the id in the lock-free
//! project snapshot and answers from the disk cache. On a miss it hands the responder to the
//! preview service, which generates the product on the job pool (interactive queue) and answers
//! every request waiting for it; no handler thread waits for a generation.
//!
//! Routes (all `GET`, by id; `v` is the media's `previewVersion`):
//! - `/poster/<mediaId>?v=<v>`: JPEG, long side 320 px, upright (video, image).
//! - `/filmstrip/<mediaId>?tile=<n>&v=<v>`: JPEG strip of the tile's frames side by side, each
//!   `frameWidth x 96` (video).
//! - `/waveform/<mediaId>?v=<v>`: binary waveform, `CFWF` layout (video with audio, audio).
//! - `/cover/<hash>`: JPEG cover of a recent project (home screen), by content hash (no `v`: the
//!   hash is the content key). From the covers folder, else copied there from the preview cache's
//!   poster; `400` unless 32 lowercase hex characters, `404` when neither has it.
//! - `/preview/frame?w=<px>&h=<px>`: one preview frame (legacy `frames` preview mode). Body: width
//!   and height as little-endian `u32`, followed by tightly packed RGBA8 rows.
//!
//! Status: `200` (immutable, cached for a year), `400` malformed id, tile or missing `v`, `404`
//! unknown id, stale `v`, a product the kind does not have or a tile out of range, `503` queue
//! full or the file unreachable with nothing cached, `500` generation failed (remembered for the
//! session).

use std::path::PathBuf;
use std::sync::Arc;

use app_core::{MediaAsset, MediaId, Project};
use crossbeam_channel::{Sender, TrySendError};
use engine::media::cache::{Product, valid_hash};
use tauri::http::{Response, StatusCode};
use tauri::{Manager, Runtime};

use crate::jobs::JobSpawner;
use crate::previews::{Key, Outcome, Previews, has_product, preview_version};
use crate::recent;
use crate::state::AppState;

pub const SCHEME: &str = "media";

/// Requests waiting for a handler thread; more are answered `503` at once.
pub const QUEUE_CAPACITY: usize = 256;
/// Threads that parse requests and serve cache hits.
pub const HANDLER_THREADS: usize = 2;

/// Answers one request, from any thread.
pub type Responder = Box<dyn FnOnce(Response<Vec<u8>>) + Send>;

/// A parsed `media://` path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Product {
        id: MediaId,
        product: Product,
        version: String,
    },
    /// A recent project's cover by content hash (checked: 32 lowercase hex characters).
    Cover {
        hash: String,
    },
    PreviewFrame,
}

/// Parses the path and query of a request URL. `Err` carries the status to answer.
pub fn parse(path: &str, query: &str) -> Result<Route, StatusCode> {
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    };
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let (kind, id) = match segments.as_slice() {
        ["preview", "frame"] => return Ok(Route::PreviewFrame),
        ["cover", hash] if valid_hash(hash) => {
            return Ok(Route::Cover {
                hash: (*hash).to_owned(),
            });
        }
        ["cover", _] => return Err(StatusCode::BAD_REQUEST),
        [kind @ ("poster" | "filmstrip" | "waveform"), id] => (*kind, *id),
        _ => return Err(StatusCode::NOT_FOUND),
    };
    let id = id
        .parse::<uuid::Uuid>()
        .map(MediaId)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let version = param("v")
        .filter(|v| !v.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_owned();
    let product = match kind {
        "poster" => Product::Poster,
        "waveform" => Product::Waveform,
        _ => Product::FilmstripTile(
            param("tile")
                .and_then(|t| t.parse::<u32>().ok())
                .ok_or(StatusCode::BAD_REQUEST)?,
        ),
    };
    Ok(Route::Product {
        id,
        product,
        version,
    })
}

/// Serves the legacy `/preview/frame` route from its query string.
pub type FrameServer = dyn Fn(&str) -> Response<Vec<u8>> + Send + Sync;

/// What the handlers need, without Tauri (tests build their own).
pub struct ServerContext {
    pub snapshot: Box<dyn Fn() -> Arc<Project> + Send + Sync>,
    pub previews: Arc<Previews>,
    pub spawner: JobSpawner,
    /// Serves `/preview/frame` (legacy `frames` mode); its query string is passed in.
    pub preview_frame: Box<FrameServer>,
    /// The recent projects' covers folder (`/cover/<hash>`).
    pub covers: PathBuf,
}

impl std::fmt::Debug for ServerContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerContext").finish_non_exhaustive()
    }
}

struct Pending {
    path: String,
    query: String,
    respond: Responder,
}

/// The bounded handler pool. Dropping it stops the handler threads once the queue is drained.
#[derive(Debug)]
pub struct MediaServer {
    queue: Sender<Pending>,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl MediaServer {
    pub fn start(context: ServerContext) -> Self {
        Self::with_limits(context, HANDLER_THREADS, QUEUE_CAPACITY)
    }

    pub fn with_limits(context: ServerContext, threads: usize, capacity: usize) -> Self {
        let (queue, requests) = crossbeam_channel::bounded::<Pending>(capacity);
        let context = Arc::new(context);
        for n in 0..threads.max(1) {
            let (requests, context) = (requests.clone(), Arc::clone(&context));
            std::thread::Builder::new()
                .name(format!("media-protocol-{n}"))
                .spawn(move || {
                    for pending in requests {
                        serve(&context, pending);
                    }
                })
                .expect("spawn a media protocol thread");
        }
        Self { queue }
    }

    /// Queues a request; answers `503` at once when the queue is full.
    pub fn submit(&self, path: String, query: String, respond: Responder) {
        let pending = Pending {
            path,
            query,
            respond,
        };
        match self.queue.try_send(pending) {
            Ok(()) => {}
            Err(TrySendError::Full(pending) | TrySendError::Disconnected(pending)) => {
                (pending.respond)(error(StatusCode::SERVICE_UNAVAILABLE, "busy"));
            }
        }
    }
}

fn find(project: &Project, id: MediaId) -> Option<&MediaAsset> {
    project.media.iter().find(|m| m.id == id)
}

fn serve(context: &ServerContext, pending: Pending) {
    let Pending {
        path,
        query,
        respond,
    } = pending;
    let route = match parse(&path, &query) {
        Ok(route) => route,
        Err(status) => return respond(error(status, "bad media route")),
    };
    let (id, product, version) = match route {
        Route::PreviewFrame => return respond((context.preview_frame)(&query)),
        Route::Cover { hash } => return respond(cover(context, &hash)),
        Route::Product {
            id,
            product,
            version,
        } => (id, product, version),
    };
    let project = (context.snapshot)();
    let Some(asset) = find(&project, id) else {
        return respond(error(StatusCode::NOT_FOUND, "unknown media"));
    };
    if version != preview_version(asset) || !has_product(asset, product) {
        return respond(error(StatusCode::NOT_FOUND, "no such preview"));
    }
    let key = Key::new(asset, product);
    if context.previews.has_failed(&key) {
        return respond(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "generation failed",
        ));
    }
    if let Some(bytes) = context.previews.cached(&key) {
        return respond(ok(product, bytes));
    }
    let waiter = Box::new(move |outcome: &Outcome| {
        respond(match outcome {
            Outcome::Ready(bytes) => ok(product, bytes.to_vec()),
            Outcome::Failed => error(StatusCode::INTERNAL_SERVER_ERROR, "generation failed"),
            Outcome::Unavailable => error(StatusCode::SERVICE_UNAVAILABLE, "media unavailable"),
        });
    });
    context
        .previews
        .request(&context.spawner, asset.clone(), product, waiter);
}

fn cover(context: &ServerContext, hash: &str) -> Response<Vec<u8>> {
    let poster = || {
        context.previews.cached(&Key {
            hash: hash.to_owned(),
            product: Product::Poster,
        })
    };
    match recent::read_cover(&context.covers, hash, poster) {
        Some(bytes) => ok(Product::Poster, bytes),
        None => error(StatusCode::NOT_FOUND, "no cover"),
    }
}

fn ok(product: Product, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", product.content_type())
        // The URL carries the content version, so a response never changes.
        .header("Cache-Control", "max-age=31536000, immutable")
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .expect("static response parts are valid")
}

/// The Tauri side: the server's context for the running app.
pub fn context(
    app: &tauri::AppHandle,
    previews: Arc<Previews>,
    spawner: JobSpawner,
    covers: PathBuf,
) -> ServerContext {
    let snapshot_app = app.clone();
    let frame_app = app.clone();
    ServerContext {
        snapshot: Box::new(move || snapshot_app.state::<AppState>().snapshot()),
        previews,
        spawner,
        preview_frame: Box::new(move |query| preview_frame(&frame_app, query)),
        covers,
    }
}

/// The protocol handler registered with Tauri: hands the request to the [`MediaServer`].
pub fn handle<R: Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let Some(server) = ctx.app_handle().try_state::<MediaServer>() else {
        responder.respond(error(StatusCode::SERVICE_UNAVAILABLE, "starting"));
        return;
    };
    let uri = request.uri();
    server.submit(
        uri.path().to_owned(),
        uri.query().unwrap_or_default().to_owned(),
        Box::new(move |response| responder.respond(response)),
    );
}

fn preview_frame<R: Runtime>(app: &tauri::AppHandle<R>, query: &str) -> Response<Vec<u8>> {
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value.parse::<u32>().ok())
    };
    let (Some(width), Some(height)) = (param("w"), param("h")) else {
        return error(StatusCode::BAD_REQUEST, "expected w and h");
    };
    let state = app.state::<AppState>();
    let Some(preview) = state.preview.as_ref() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "preview unavailable");
    };
    let Some(frame) = preview.render_frame(width, height) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "preview did not answer");
    };

    let mut body = Vec::with_capacity(8 + frame.rgba.len());
    body.extend_from_slice(&frame.width.to_le_bytes());
    body.extend_from_slice(&frame.height.to_le_bytes());
    body.extend_from_slice(&frame.rgba);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .expect("static response parts are valid")
}

fn error(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(message.as_bytes().to_vec())
        .expect("static response parts are valid")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;

    use app_core::media::samples;

    use super::*;
    use crate::jobs::JobManager;
    use crate::jobs::tests::FakeHost;
    use crate::previews::tests::{fake_generator, temp_cache};

    #[test]
    fn routes_parse_by_id_with_version() {
        let id = MediaId::new();
        let route = |path: String, query: &str| parse(&path, query);
        assert_eq!(
            route(format!("/poster/{}", id.0), "v=abc"),
            Ok(Route::Product {
                id,
                product: Product::Poster,
                version: "abc".into()
            })
        );
        assert_eq!(
            route(format!("/filmstrip/{}", id.0), "tile=3&v=abc"),
            Ok(Route::Product {
                id,
                product: Product::FilmstripTile(3),
                version: "abc".into()
            })
        );
        assert_eq!(
            route(format!("/waveform/{}", id.0), "v=x").map(|r| matches!(
                r,
                Route::Product {
                    product: Product::Waveform,
                    ..
                }
            )),
            Ok(true)
        );
        assert_eq!(parse("/preview/frame", "w=1&h=1"), Ok(Route::PreviewFrame));
        let hash = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            parse(&format!("/cover/{hash}"), ""),
            Ok(Route::Cover { hash: hash.into() })
        );
        for bad_hash in ["0123", "0123456789ABCDEF0123456789ABCDEF", "..", ""] {
            assert_eq!(
                parse(&format!("/cover/{bad_hash}"), ""),
                Err(StatusCode::BAD_REQUEST),
                "{bad_hash}"
            );
        }

        let bad = StatusCode::BAD_REQUEST;
        assert_eq!(parse("/poster/not-a-uuid", "v=a"), Err(bad));
        assert_eq!(
            route(format!("/poster/{}", id.0), ""),
            Err(bad),
            "v is required"
        );
        assert_eq!(route(format!("/poster/{}", id.0), "v="), Err(bad));
        assert_eq!(
            route(format!("/filmstrip/{}", id.0), "v=a"),
            Err(bad),
            "tile is required"
        );
        assert_eq!(
            route(format!("/filmstrip/{}", id.0), "v=a&tile=-1"),
            Err(bad)
        );
        assert_eq!(
            route(format!("/filmstrip/{}", id.0), "v=a&tile=x"),
            Err(bad)
        );

        let missing = StatusCode::NOT_FOUND;
        assert_eq!(parse("/", ""), Err(missing));
        assert_eq!(route(format!("/thumbnail/{}", id.0), "v=a"), Err(missing));
        assert_eq!(
            route(format!("/poster/{}/extra", id.0), "v=a"),
            Err(missing)
        );
        assert_eq!(
            parse("/poster/C:/Users/me/a.mp4", "v=a"),
            Err(missing),
            "never by path"
        );
    }

    struct Fixture {
        server: MediaServer,
        calls: Arc<Mutex<Vec<Product>>>,
        project: Arc<Mutex<Arc<Project>>>,
        _jobs: JobManager,
        root: std::path::PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    fn fixture(name: &str, media: Vec<MediaAsset>, delay: Duration) -> Fixture {
        let calls: Arc<Mutex<Vec<Product>>> = Arc::default();
        let (root, cache) = temp_cache(name);
        let previews = Arc::new(Previews::with_generator(
            Some(cache),
            fake_generator(Arc::clone(&calls), delay),
        ));
        let (host, _rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 2);
        let mut project = Project::new("Test");
        project.media = media;
        let project = Arc::new(Mutex::new(Arc::new(project)));
        let snapshot = Arc::clone(&project);
        let server = MediaServer::with_limits(
            ServerContext {
                snapshot: Box::new(move || Arc::clone(&snapshot.lock().unwrap())),
                previews,
                spawner: jobs.spawner(),
                preview_frame: Box::new(|_| error(StatusCode::SERVICE_UNAVAILABLE, "no preview")),
                covers: root.join("covers"),
            },
            2,
            QUEUE_CAPACITY,
        );
        Fixture {
            server,
            calls,
            project,
            _jobs: jobs,
            root,
        }
    }

    fn get(server: &MediaServer, url: &str) -> mpsc::Receiver<Response<Vec<u8>>> {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let (tx, rx) = mpsc::channel();
        server.submit(
            path.to_owned(),
            query.to_owned(),
            Box::new(move |response| {
                tx.send(response).ok();
            }),
        );
        rx
    }

    fn status(rx: &mpsc::Receiver<Response<Vec<u8>>>) -> (StatusCode, Vec<u8>) {
        let response = rx.recv_timeout(Duration::from_secs(10)).expect("an answer");
        (response.status(), response.into_body())
    }

    fn url(kind: &str, asset: &MediaAsset, extra: &str) -> String {
        format!("/{kind}/{}?{extra}v={}", asset.id.0, preview_version(asset))
    }

    #[test]
    fn products_are_served_by_id_and_cached() {
        let video = samples::video(1);
        let image = samples::image(2);
        let f = fixture("serve", vec![video.clone(), image.clone()], Duration::ZERO);

        let rx = get(&f.server, &url("poster", &video, ""));
        let response = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["Content-Type"], "image/jpeg");
        assert_eq!(
            response.headers()["Cache-Control"],
            "max-age=31536000, immutable"
        );
        let body = response.into_body();
        assert_eq!(
            body,
            format!("Poster@{}", video.fingerprint.hash).into_bytes()
        );

        // A second request is a cache hit: nothing new is generated.
        assert_eq!(
            status(&get(&f.server, &url("poster", &video, ""))),
            (StatusCode::OK, body)
        );
        assert_eq!(f.calls.lock().unwrap().len(), 1);

        let (code, _) = status(&get(&f.server, &url("waveform", &video, "")));
        assert_eq!(code, StatusCode::OK);
        let (code, _) = status(&get(&f.server, &url("filmstrip", &video, "tile=0&")));
        assert_eq!(code, StatusCode::OK);
    }

    #[test]
    fn unknown_ids_stale_versions_and_missing_products_are_not_found() {
        let video = samples::video(1);
        let image = samples::image(2);
        let f = fixture(
            "missing",
            vec![video.clone(), image.clone()],
            Duration::ZERO,
        );
        let not_found = |url: String| {
            assert_eq!(
                status(&get(&f.server, &url)).0,
                StatusCode::NOT_FOUND,
                "{url}"
            );
        };
        not_found(url("poster", &samples::video(9), ""));
        not_found(format!("/poster/{}?v=0000000000000000", video.id.0));
        not_found(url("waveform", &image, ""));
        not_found(url("filmstrip", &image, "tile=0&"));
        not_found(url("filmstrip", &video, "tile=1&"));
        assert_eq!(
            status(&get(&f.server, "/poster/123?v=a")).0,
            StatusCode::BAD_REQUEST
        );
        assert!(f.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn two_concurrent_misses_share_one_generation() {
        let video = samples::video(1);
        let f = fixture(
            "concurrent",
            vec![video.clone()],
            Duration::from_millis(300),
        );
        let first = get(&f.server, &url("poster", &video, ""));
        let second = get(&f.server, &url("poster", &video, ""));
        let a = status(&first);
        let b = status(&second);
        assert_eq!(a.0, StatusCode::OK);
        assert_eq!(a, b);
        assert_eq!(*f.calls.lock().unwrap(), [Product::Poster]);
    }

    #[test]
    fn offline_media_serves_its_cached_previews_and_is_unavailable_otherwise() {
        let video = samples::video(1);
        let f = fixture("offline", vec![video.clone()], Duration::ZERO);
        assert_eq!(
            status(&get(&f.server, &url("poster", &video, ""))).0,
            StatusCode::OK
        );

        // The file goes missing (renamed path in the model stands in for "offline").
        let mut gone = video.clone();
        gone.path.absolute = "C:\\missing\\clip.mp4".into();
        {
            let mut project = (**f.project.lock().unwrap()).clone();
            project.media = vec![gone.clone()];
            *f.project.lock().unwrap() = Arc::new(project);
        }
        // Cached by content: still served.
        assert_eq!(
            status(&get(&f.server, &url("poster", &gone, ""))).0,
            StatusCode::OK
        );
        // Not cached: unavailable, and not remembered as a failure.
        assert_eq!(
            status(&get(&f.server, &url("waveform", &gone, ""))).0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status(&get(&f.server, &url("waveform", &gone, ""))).0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(f.calls.lock().unwrap().len(), 3, "retried");
    }

    #[test]
    fn failed_generation_is_remembered() {
        let mut broken = samples::video(1);
        broken.path.absolute = "C:\\broken.mp4".into();
        let f = fixture("failed", vec![broken.clone()], Duration::ZERO);
        let error = StatusCode::INTERNAL_SERVER_ERROR;
        assert_eq!(
            status(&get(&f.server, &url("poster", &broken, ""))).0,
            error
        );
        assert_eq!(
            status(&get(&f.server, &url("poster", &broken, ""))).0,
            error
        );
        assert_eq!(f.calls.lock().unwrap().len(), 1, "not retried in a loop");
    }

    #[test]
    fn a_full_queue_answers_service_unavailable() {
        let video = samples::video(1);
        let (root, cache) = temp_cache("full");
        let previews = Arc::new(Previews::with_generator(
            Some(cache),
            fake_generator(Arc::default(), Duration::ZERO),
        ));
        let (host, _rx) = FakeHost::new();
        let jobs = JobManager::with_workers(host, 1);
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate_rx = Mutex::new(gate_rx);
        let mut project = Project::new("Test");
        project.media = vec![video.clone()];
        let project = Arc::new(project);
        // The only handler blocks in the snapshot call of the first request.
        let server = MediaServer::with_limits(
            ServerContext {
                snapshot: Box::new(move || {
                    gate_rx.lock().unwrap().recv().ok();
                    Arc::clone(&project)
                }),
                previews,
                spawner: jobs.spawner(),
                preview_frame: Box::new(|_| error(StatusCode::SERVICE_UNAVAILABLE, "no preview")),
                covers: root.join("covers"),
            },
            1,
            2,
        );
        let poster = url("poster", &video, "");
        let first = get(&server, &poster);
        // Let the handler pick up the first request, then fill the queue.
        std::thread::sleep(Duration::from_millis(100));
        let queued: Vec<_> = (0..2).map(|_| get(&server, &poster)).collect();
        let rejected = get(&server, &poster);
        assert_eq!(status(&rejected).0, StatusCode::SERVICE_UNAVAILABLE);
        for _ in 0..3 {
            gate_tx.send(()).unwrap();
        }
        assert_eq!(status(&first).0, StatusCode::OK);
        for rx in &queued {
            assert_eq!(status(rx).0, StatusCode::OK);
        }
        std::fs::remove_dir_all(root).ok();
    }
    #[test]
    fn covers_are_served_from_the_covers_folder_then_the_preview_cache() {
        let video = samples::video(1);
        let f = fixture("covers", vec![video.clone()], Duration::ZERO);
        let hash = &video.fingerprint.hash;
        let covers = f.root.join("covers");
        let cover = |hash: &str| status(&get(&f.server, &format!("/cover/{hash}")));

        // Neither a cover nor a cached poster.
        assert_eq!(cover(hash).0, StatusCode::NOT_FOUND);
        assert_eq!(cover("bad").0, StatusCode::BAD_REQUEST);

        // The poster gets cached; the cover is copied from it.
        let (code, poster) = status(&get(&f.server, &url("poster", &video, "")));
        assert_eq!(code, StatusCode::OK);
        assert_eq!(cover(hash), (StatusCode::OK, poster.clone()));
        assert_eq!(
            std::fs::read(covers.join(format!("{hash}.jpg"))).unwrap(),
            poster
        );

        // A cover hit needs no cache (a cleared cache keeps the home screen's covers).
        let other = "abcdefabcdefabcdefabcdefabcdefab";
        std::fs::write(covers.join(format!("{other}.jpg")), b"kept").unwrap();
        assert_eq!(cover(other), (StatusCode::OK, b"kept".to_vec()));
    }
}
