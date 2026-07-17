//! Embedded HTTP server for the forgetop web dashboard.
//!
//! It's a thin frontend over the **same** `SectionService` / health services the TUI uses —
//! no logic fork. Wave 1 is a read-only JSON API + a placeholder page, bound to `127.0.0.1`
//! and gated by a per-session token so no other local process or web page can reach it.

use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::extract::{Query, Request, State};
use axum::http::{header, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use forgetop_core::secret::SecretStore;
use forgetop_core::service::{ConfigService, ConnectionHealthService, SectionService};
use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::actions::ActionError;

mod actions;
mod connections;
mod dto;

/// The built dashboard SPA, baked into the binary at compile time (see `build.rs`).
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Default dashboard port. `0` lets the OS pick a free one.
pub const DEFAULT_PORT: u16 = 8177;

/// The services the dashboard reads from and writes through — the same ones the TUI is given,
/// so connections added here show up in the TUI (and vice versa).
#[derive(Clone)]
pub struct Deps {
    pub sections: Arc<SectionService>,
    pub health: Arc<ConnectionHealthService>,
    pub config: Arc<ConfigService>,
    pub secrets: Arc<dyn SecretStore>,
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
        .route("/api/pr/detail", get(pr_detail))
        .route("/api/pr/vote", post(pr_vote))
        .route("/api/pr/merge", post(pr_merge))
        .route("/api/pr/revert", post(pr_revert))
        .route("/api/pr/comment", post(pr_comment))
        .route("/api/pr/review", post(pr_review))
        .route("/api/wi/detail", get(wi_detail))
        .route("/api/wi/states", get(wi_states))
        .route("/api/wi/state", post(wi_state))
        .route("/api/wi/comment", post(wi_comment))
        .route("/api/pipeline/detail", get(pipeline_detail))
        .route("/api/pipeline/logs", get(pipeline_logs))
        .route("/api/pipeline/approval", post(pipeline_approval))
        .route("/api/pipeline/trigger", post(pipeline_trigger))
        .route("/api/notification/read", post(notification_read))
        .route("/api/providers", get(providers))
        .route("/api/connections", get(list_connections).post(save_connection))
        .route("/api/connections/delete", post(delete_connection))
        .route("/api/connections/test", post(test_connection))
        .route("/api/preferences", get(get_preferences))
        .route("/api/preferences/startup", post(set_startup_mode))
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
async fn pull_requests(State(s): State<AppState>, Query(q): Query<PrListQuery>) -> Json<Vec<dto::PrRow>> {
    Json(dto::pull_requests(&s.deps.sections, dto::PrView::parse(q.view.as_deref())).await)
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
async fn launchpad(State(s): State<AppState>) -> Json<dto::LaunchpadResponse> {
    Json(dto::launchpad(&s.deps.sections).await)
}

/// Query params identifying an item within a connection (`?conn=…&id=…`).
#[derive(Deserialize)]
struct ItemQuery {
    conn: String,
    id: String,
}

/// Query params for the PR list: which view to show (`?view=all|merged|review_requested`).
#[derive(Deserialize)]
struct PrListQuery {
    #[serde(default)]
    view: Option<String>,
}

/// Query params identifying a pipeline run within a connection (`?conn=…&run_id=…`).
#[derive(Deserialize)]
struct RunQuery {
    conn: String,
    run_id: String,
}

/// Query params for pipeline logs: a run, optionally scoped to a single job (`&job=…`).
#[derive(Deserialize)]
struct PipelineLogsQuery {
    conn: String,
    run_id: String,
    #[serde(default)]
    job: Option<String>,
}

/// Turns an action outcome into a response: `{ok:true}`, 404 (no such connection/capability),
/// or 502 (the provider call failed).
fn action_response(result: Result<(), ActionError>) -> Response {
    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(ActionError::NotFound) => (StatusCode::NOT_FOUND, "connection or capability not found").into_response(),
        Err(ActionError::Failed(msg)) => (StatusCode::BAD_GATEWAY, msg).into_response(),
    }
}

async fn pr_detail(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    match dto::pr_detail(&s.deps.sections, &q.conn, &q.id).await {
        Some(detail) => Json(detail).into_response(),
        None => (StatusCode::NOT_FOUND, "pull request not found").into_response(),
    }
}

async fn pr_vote(State(s): State<AppState>, Json(req): Json<actions::PrVoteReq>) -> Response {
    action_response(actions::pr_vote(&s.deps.sections, req).await)
}
async fn pr_merge(State(s): State<AppState>, Json(req): Json<actions::PrMergeReq>) -> Response {
    action_response(actions::pr_merge(&s.deps.sections, req).await)
}
async fn pr_revert(State(s): State<AppState>, Json(req): Json<actions::PrRevertReq>) -> Response {
    action_response(actions::pr_revert(&s.deps.sections, req).await)
}
async fn pr_comment(State(s): State<AppState>, Json(req): Json<actions::PrCommentReq>) -> Response {
    action_response(actions::pr_comment(&s.deps.sections, req).await)
}
async fn pr_review(State(s): State<AppState>, Json(req): Json<actions::PrReviewReq>) -> Response {
    action_response(actions::pr_review(&s.deps.sections, req).await)
}

async fn wi_detail(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    match dto::wi_detail(&s.deps.sections, &q.conn, &q.id).await {
        Some(detail) => Json(detail).into_response(),
        None => (StatusCode::NOT_FOUND, "work item not found").into_response(),
    }
}
async fn wi_states(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    match actions::wi_states(&s.deps.sections, &q.conn, &q.id).await {
        Some(states) => Json(states).into_response(),
        None => (StatusCode::NOT_FOUND, "work item connection not found").into_response(),
    }
}
async fn wi_state(State(s): State<AppState>, Json(req): Json<actions::WiStateReq>) -> Response {
    action_response(actions::wi_set_state(&s.deps.sections, req).await)
}
async fn wi_comment(State(s): State<AppState>, Json(req): Json<actions::WiCommentReq>) -> Response {
    action_response(actions::wi_comment(&s.deps.sections, req).await)
}

async fn pipeline_detail(State(s): State<AppState>, Query(q): Query<RunQuery>) -> Response {
    match dto::pipeline_detail(&s.deps.sections, &q.conn, &q.run_id).await {
        Some(detail) => Json(detail).into_response(),
        None => (StatusCode::NOT_FOUND, "pipeline run not found").into_response(),
    }
}
async fn pipeline_logs(State(s): State<AppState>, Query(q): Query<PipelineLogsQuery>) -> Response {
    match dto::pipeline_logs(&s.deps.sections, &q.conn, &q.run_id, q.job.as_deref()).await {
        Some(text) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response(),
        None => (StatusCode::NOT_FOUND, "logs not available").into_response(),
    }
}

async fn pipeline_approval(State(s): State<AppState>, Json(req): Json<actions::PipelineApprovalReq>) -> Response {
    action_response(actions::pipeline_approval(&s.deps.sections, req).await)
}
async fn pipeline_trigger(State(s): State<AppState>, Json(req): Json<actions::PipelineTriggerReq>) -> Response {
    action_response(actions::pipeline_trigger(&s.deps.sections, req).await)
}

async fn notification_read(State(s): State<AppState>, Json(req): Json<actions::NotifReadReq>) -> Response {
    action_response(actions::notif_read(&s.deps.sections, req).await)
}

// ---- connection management ----

#[derive(Deserialize)]
struct IdReq {
    id: String,
}

async fn providers() -> Json<Vec<connections::ProviderInfo>> {
    Json(connections::providers())
}
async fn list_connections(State(s): State<AppState>) -> Json<Vec<connections::ConnectionRow>> {
    Json(connections::list(&s.deps.config, s.deps.secrets.as_ref()))
}
async fn save_connection(State(s): State<AppState>, Json(req): Json<connections::SaveConnectionReq>) -> Response {
    match connections::save(&s.deps.config, req).await {
        Ok(id) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(msg) => (StatusCode::BAD_GATEWAY, msg).into_response(),
    }
}
async fn delete_connection(State(s): State<AppState>, Json(req): Json<IdReq>) -> Response {
    match connections::remove(&s.deps.config, &req.id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(msg) => (StatusCode::BAD_GATEWAY, msg).into_response(),
    }
}
async fn test_connection(State(s): State<AppState>, Json(req): Json<IdReq>) -> Response {
    match connections::test(&s.deps.health, &req.id).await {
        Some(healthy) => Json(serde_json::json!({ "healthy": healthy })).into_response(),
        None => (StatusCode::NOT_FOUND, "connection not found").into_response(),
    }
}

#[derive(Deserialize)]
struct StartupModeReq {
    mode: forgetop_core::config::StartupMode,
}

/// User preferences shared with the TUI (currently just the startup mode).
async fn get_preferences(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "startup_mode": s.deps.config.snapshot().ui.startup_mode }))
}
async fn set_startup_mode(State(s): State<AppState>, Json(req): Json<StartupModeReq>) -> Response {
    match s.deps.config.set_startup_mode(req.mode).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}
