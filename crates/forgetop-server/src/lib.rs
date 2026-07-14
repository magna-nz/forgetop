//! Embedded HTTP server for the forgetop web dashboard.
//!
//! It's a thin frontend over the **same** `SectionService` / health services the TUI uses —
//! no logic fork. Wave 1 is a read-only JSON API + a placeholder page, bound to `127.0.0.1`
//! and gated by a per-session token so no other local process or web page can reach it.

use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use forgetop_core::service::{ConnectionHealthService, SectionService};

mod dto;

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
    Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/api/pull-requests", get(pull_requests))
        .route("/api/work-items", get(work_items))
        .route("/api/pipelines", get(pipelines))
        .route("/api/notifications", get(notifications))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
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

async fn index() -> Html<&'static str> {
    Html(PLACEHOLDER)
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

const PLACEHOLDER: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>forgetop dashboard</title>
<style>
  body{font-family:ui-monospace,"SF Mono",Menlo,monospace;background:#12151b;color:#e6e9ef;max-width:760px;margin:48px auto;padding:0 20px;line-height:1.6}
  h1{color:#6fb0ff;font-weight:600} code{background:#1b2027;padding:2px 6px;border-radius:5px;color:#e6e9ef}
  a{color:#6bd39a;text-decoration:none} a:hover{text-decoration:underline}
  .dim{color:#8b95a5} .ok{color:#6bd39a} ul{padding-left:20px}
</style></head><body>
  <h1>&#9727; forgetop dashboard</h1>
  <p class="dim">Wave 1 &mdash; the server is up. The React app arrives in wave 2.</p>
  <p>Read-only API (send <code>?t=&lt;token&gt;</code> or the <code>x-forgetop-token</code> header):</p>
  <ul>
    <li><a>/api/health</a></li>
    <li><a>/api/pull-requests</a></li>
    <li><a>/api/work-items</a></li>
    <li><a>/api/pipelines</a></li>
    <li><a>/api/notifications</a></li>
  </ul>
  <p>Live check: <span id="probe" class="dim">probing&hellip;</span></p>
  <script>
    const t = new URLSearchParams(location.search).get('t') || '';
    for (const a of document.querySelectorAll('ul a')) a.href = a.textContent + '?t=' + t;
    fetch('/api/health?t=' + t).then(r => r.json()).then(h => {
      document.getElementById('probe').innerHTML = '<span class="ok">/api/health OK</span> — ' + h.length + ' connection(s)';
    }).catch(e => { document.getElementById('probe').textContent = 'probe failed: ' + e; });
  </script>
</body></html>"#;
