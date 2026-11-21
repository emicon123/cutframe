//! The `media://` protocol: binary assets for the UI that must not go through JSON IPC.
//!
//! Routes:
//! - `/preview/frame?w=<px>&h=<px>`: one preview frame (`frames` preview mode). Body: width and
//!   height as little-endian `u32`, followed by tightly packed RGBA8 rows.
//!
//! Thumbnails and waveforms will be added here.

use tauri::http::{Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::state::AppState;

pub const SCHEME: &str = "media";

pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    // Rendering blocks; never do it on the webview's thread.
    std::thread::spawn(move || {
        let response = route(&app, &request);
        responder.respond(response);
    });
}

fn route<R: Runtime>(app: &tauri::AppHandle<R>, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    match request.uri().path() {
        "/preview/frame" => preview_frame(app, request.uri().query().unwrap_or_default()),
        _ => error(StatusCode::NOT_FOUND, "not found"),
    }
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
        .header("Access-Control-Allow-Origin", "*")
        .body(message.as_bytes().to_vec())
        .expect("static response parts are valid")
}
