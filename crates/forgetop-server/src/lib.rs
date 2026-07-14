//! Embedded HTTP server for the forgetop web dashboard.
//!
//! It's a thin frontend over the **same** `SectionService` / health services the TUI uses —
//! no logic fork. Wave 1 is a read-only JSON API + a placeholder page, bound to `127.0.0.1`
//! and gated by a per-session token so no other local process or web page can reach it.

use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use forgetop_core::service::{ConnectionHealthService, SectionService};
use rust_embed::RustEmbed;

mod dto;

/// The built dashboard SPA, baked into the binary at compile time (see `build.rs`).
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Default dashboard port. `0` lets the OS pick a free one.
pub const DEFAULT_PORT: u16 = 8177;

/// The services the dashboard reads from — the same ones the TUI is given.
#[derive(Clone)]
pub struct Deps {
    pub sections: Arc<SectionService>,
    pub health: Arc<ConnectionHealthService>,
}

/// A bound, running server: where it lives and the token needed to reach it.
pub struct Server {
    pub port: u16,
    pub token: String,
    /// `http://127.0.0.1:<port>/?t=<token>` — the URL to open in a browser.
    pub url: String,
}

#[derive(Clone)]
struct AppState {
    deps: Deps,
    token: Arc<str>,
}

async fn bind(deps: Deps, port: u16) -> std::io::Result<(tokio::net::TcpListener, Server, AppState)> {
    let token = uuid::Uuid::new_v4().to_string();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let bound = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{bound}/?t={token}");
    let state = AppState { deps, token: Arc::from(token.as_str()) };
    Ok((listener, Server { port: bound, token, url }, state))
}

/// Bind + serve in the background, returning the URL (with token) for the caller to open.
/// **Best-effort:** an `Err` just means "no dashboard" — the TUI should carry on regardless.
pub async fn spawn(deps: Deps, port: u16) -> std::io::Result<Server> {
    let (listener, server, state) = bind(deps, port).await?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });
    Ok(server)
}

/// Bind + serve until the process exits (headless `--dashboard`). Calls `on_ready` with the
/// URL once bound so the caller can open the browser.
pub async fn serve_blocking(deps: Deps, port: u16, on_ready: impl FnOnce(&str)) -> std::io::Result<()> {
    let (listener, server, state) = bind(deps, port).await?;
    on_ready(&server.url);
    axum::serve(listener, router(state)).await
}

fn router(state: AppState) -> Router {
    // The API carries your data and can act on your behalf, so it's token-gated. The static
    // SPA (HTML/JS/CSS) is just code — not secret — so it's served openly; the browser gets
    // the token from the `/?t=` URL and replays it on every API call.
    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/pull-requests", get(pull_requests))
        .route("/api/work-items", get(work_items))
        .route("/api/pipelines", get(pipelines))
        .route("/api/notifications", get(notifications))
        .route("/api/launchpad", get(launchpad))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state);

    Router::new().merge(api).fallback(static_asset)
}

/// Serves an embedded SPA asset by path, falling back to `index.html` for unknown routes so
/// client-side routing works on refresh/deep-link.
async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    serve(path)
        .or_else(|| serve("index.html"))
        .unwrap_or_else(|| (StatusCode::NOT_FOUND, "not found").into_response())
}

fn serve(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Some(([(header::CONTENT_TYPE, mime.as_ref().to_string())], file.data).into_response())
}

/// Session-token gate. The browser opens `/?t=<token>`; other calls may send it as the
/// `x-forgetop-token` header. Combined with localhost-only binding, this keeps the
/// (action-capable) API off-limits to other local processes and web pages.
async fn auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if token_from(&req).as_deref() == Some(&state.token) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

fn token_from(req: &Request) -> Option<String> {
    if let Some(header) = req.headers().get("x-forgetop-token").and_then(|v| v.to_str().ok()) {
        return Some(header.to_string());
    }
    req.uri()
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("t=").map(str::to_string)))
}

async fn health(State(s): State<AppState>) -> Json<Vec<dto::HealthRow>> {
    Json(dto::health(&s.deps.health).await)
}
async fn pull_requests(State(s): State<AppState>) -> Json<Vec<dto::PrRow>> {
    Json(dto::pull_requests(&s.deps.sections).await)
}
async fn work_items(State(s): State<AppState>) -> Json<Vec<dto::WiRow>> {
    Json(dto::work_items(&s.deps.sections).await)
}
async fn pipelines(State(s): State<AppState>) -> Json<Vec<dto::PipeRow>> {
    Json(dto::pipelines(&s.deps.sections).await)
}
async fn notifications(State(s): State<AppState>) -> Json<Vec<dto::NotifRow>> {
    Json(dto::notifications(&s.deps.sections).await)
}
async fn launchpad(State(s): State<AppState>) -> Json<Vec<dto::LaunchpadRow>> {
    Json(dto::launchpad(&s.deps.sections).await)
}
