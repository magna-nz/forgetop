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
use forgetop_core::provider::{ItemRef, PrDecoration};
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

/// Decorated PR fields, cached per `(connection, repository, id, updated_at)`.
///
/// `updated_at` is part of the key on purpose: it is the provider's own statement that the PR
/// changed, so a stale entry can never outlive the change that invalidates it.
#[derive(Default)]
struct DecorationCache {
    entries: std::sync::Mutex<std::collections::HashMap<String, PrDecoration>>,
}

impl DecorationCache {
    /// Bounded so a long-running dashboard can't grow it without limit. On overflow the whole
    /// map is dropped rather than evicted one by one — decoration is cheap to refetch.
    const MAX: usize = 500;

    fn key(conn: &str, item: &ItemRef, updated_at: Option<&str>) -> String {
        format!("{conn}\u{1}{}\u{1}{}\u{1}{}", item.repo.as_deref().unwrap_or(""), item.id, updated_at.unwrap_or(""))
    }

    fn get(&self, key: &str) -> Option<PrDecoration> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    fn put(&self, key: String, value: PrDecoration) {
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= Self::MAX {
            entries.clear();
        }
        entries.insert(key, value);
    }
}

#[derive(Clone)]
struct AppState {
    deps: Deps,
    token: Arc<str>,
    decorations: Arc<DecorationCache>,
}

async fn bind(deps: Deps, port: u16) -> std::io::Result<(tokio::net::TcpListener, Server, AppState)> {
    let token = uuid::Uuid::new_v4().to_string();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let bound = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{bound}/?t={token}");
    let state = AppState { deps, token: Arc::from(token.as_str()), decorations: Arc::new(DecorationCache::default()) };
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
        .route("/api/pr/decoration", get(pr_decoration))
        .route("/api/pr/commit-changes", get(pr_commit_changes))
        .route("/api/pr/vote", post(pr_vote))
        .route("/api/pr/merge", post(pr_merge))
        .route("/api/pr/revert", post(pr_revert))
        .route("/api/pr/comment", post(pr_comment))
        .route("/api/pr/reply", post(pr_reply))
        .route("/api/pr/review", post(pr_review))
        .route("/api/pr/resolve-thread", post(pr_resolve_thread))
        .route("/api/pr/draft", post(pr_draft))
        .route("/api/pr/closed", post(pr_closed))
        .route("/api/pr/reviewers", get(pr_reviewers))
        .route("/api/pr/request-reviewer", post(pr_request_reviewer))
        .route("/api/wi/detail", get(wi_detail))
        .route("/api/wi/states", get(wi_states))
        .route("/api/wi/state", post(wi_state))
        .route("/api/wi/comment", post(wi_comment))
        .route("/api/wi/assignees", get(wi_assignees))
        .route("/api/wi/assignee", post(wi_assignee))
        .route("/api/wi/update", post(wi_update))
        .route("/api/pipeline/detail", get(pipeline_detail))
        .route("/api/pipeline/logs", get(pipeline_logs))
        .route("/api/pipeline/approval", post(pipeline_approval))
        .route("/api/pipeline/trigger", post(pipeline_trigger))
        .route("/api/pipeline/cancel", post(pipeline_cancel))
        .route("/api/notification/read", post(notification_read))
        .route("/api/providers", get(providers))
        .route("/api/connections", get(list_connections).post(save_connection))
        .route("/api/connections/delete", post(delete_connection))
        .route("/api/connections/test", post(test_connection))
        .route("/api/connections/repositories", get(connection_repositories))
        .route("/api/connections/scope", post(set_connection_scope))
        .route("/api/pipelines/definitions", get(pipeline_definitions))
        .route("/api/pipelines/selection", post(set_pipeline_selection))
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

/// Query params identifying an item within a connection (`?conn=…&id=…&repo=…`).
///
/// `repo` is a **query parameter**, never a path segment: an `owner/repo` always contains a
/// slash. It is optional, so every link written before connections spanned an account still
/// resolves (a single-repository connection needs no address).
#[derive(Deserialize)]
struct ItemQuery {
    conn: String,
    id: String,
    #[serde(default)]
    repo: Option<String>,
}

impl ItemQuery {
    fn item(self) -> (String, ItemRef) {
        (self.conn, ItemRef::maybe(self.repo, self.id))
    }
}

/// Query params identifying a single commit's diff within a PR (`?conn=…&id=…&sha=…`).
#[derive(Deserialize)]
struct CommitQuery {
    conn: String,
    id: String,
    sha: String,
    #[serde(default)]
    repo: Option<String>,
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
    #[serde(default)]
    repo: Option<String>,
}

/// Query params for pipeline logs: a run, optionally scoped to a single job (`&job=…`).
#[derive(Deserialize)]
struct PipelineLogsQuery {
    conn: String,
    run_id: String,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    job: Option<String>,
}

/// Turns an action outcome into a response: `{ok:true}`, 404 (no such connection/capability),
/// or 502 (the provider call failed).
fn action_response(operation: &'static str, result: Result<(), ActionError>) -> Response {
    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(ActionError::NotFound) => (StatusCode::NOT_FOUND, "connection or capability not found").into_response(),
        Err(ActionError::Failed(msg)) => {
            forgetop_core::diag::log(operation, "provider action failed");
            (StatusCode::BAD_GATEWAY, msg).into_response()
        }
    }
}

async fn pr_commit_changes(State(s): State<AppState>, Query(q): Query<CommitQuery>) -> Response {
    match dto::pr_commit_changes(&s.deps.sections, &q.conn, &ItemRef::maybe(q.repo, q.id), &q.sha).await {
        Some(changes) => Json(changes).into_response(),
        None => (StatusCode::NOT_FOUND, "pull request not found").into_response(),
    }
}

async fn pr_detail(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    let (conn, item) = q.item();
    match dto::pr_detail(&s.deps.sections, &conn, &item).await {
        Some(detail) => Json(detail).into_response(),
        None => (StatusCode::NOT_FOUND, "pull request not found").into_response(),
    }
}

/// Query params for one PR's decoration. `updated_at` is the provider's own last-changed stamp
/// from the list row, and is what makes the cache entry safe to reuse.
#[derive(Deserialize)]
struct DecorationQuery {
    conn: String,
    id: String,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

async fn pr_decoration(State(s): State<AppState>, Query(q): Query<DecorationQuery>) -> Response {
    let item = ItemRef::maybe(q.repo, q.id);
    let key = DecorationCache::key(&q.conn, &item, q.updated_at.as_deref());
    if let Some(hit) = s.decorations.get(&key) {
        return Json(hit).into_response();
    }
    match dto::pr_decoration(&s.deps.sections, &q.conn, &item).await {
        Some(decoration) => {
            s.decorations.put(key, decoration.clone());
            Json(decoration).into_response()
        }
        None => (StatusCode::NOT_FOUND, "pull request not found").into_response(),
    }
}

async fn pr_vote(State(s): State<AppState>, Json(req): Json<actions::PrVoteReq>) -> Response {
    action_response("dashboard.pr_vote", actions::pr_vote(&s.deps.sections, req).await)
}
async fn pr_merge(State(s): State<AppState>, Json(req): Json<actions::PrMergeReq>) -> Response {
    action_response("dashboard.pr_merge", actions::pr_merge(&s.deps.sections, req).await)
}
async fn pr_revert(State(s): State<AppState>, Json(req): Json<actions::PrRevertReq>) -> Response {
    action_response("dashboard.pr_revert", actions::pr_revert(&s.deps.sections, req).await)
}
async fn pr_comment(State(s): State<AppState>, Json(req): Json<actions::PrCommentReq>) -> Response {
    action_response("dashboard.pr_comment", actions::pr_comment(&s.deps.sections, req).await)
}
async fn pr_reply(State(s): State<AppState>, Json(req): Json<actions::PrReplyReq>) -> Response {
    action_response("dashboard.pr_reply", actions::pr_reply(&s.deps.sections, req).await)
}
async fn pr_review(State(s): State<AppState>, Json(req): Json<actions::PrReviewReq>) -> Response {
    action_response("dashboard.pr_review", actions::pr_review(&s.deps.sections, req).await)
}
async fn pr_resolve_thread(State(s): State<AppState>, Json(req): Json<actions::PrResolveThreadReq>) -> Response {
    action_response("dashboard.pr_resolve_thread", actions::pr_resolve_thread(&s.deps.sections, req).await)
}
async fn pr_draft(State(s): State<AppState>, Json(req): Json<actions::PrDraftReq>) -> Response {
    action_response("dashboard.pr_draft", actions::pr_set_draft(&s.deps.sections, req).await)
}
async fn pr_closed(State(s): State<AppState>, Json(req): Json<actions::PrClosedReq>) -> Response {
    action_response("dashboard.pr_closed", actions::pr_set_closed(&s.deps.sections, req).await)
}
async fn pr_reviewers(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    let (conn, item) = q.item();
    match actions::pr_reviewers(&s.deps.sections, &conn, &item).await {
        Some(users) => Json(users).into_response(),
        None => (StatusCode::NOT_FOUND, "pull request connection not found").into_response(),
    }
}
async fn pr_request_reviewer(State(s): State<AppState>, Json(req): Json<actions::PrRequestReviewerReq>) -> Response {
    action_response("dashboard.pr_request_reviewer", actions::pr_request_reviewer(&s.deps.sections, req).await)
}

async fn wi_detail(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    let (conn, item) = q.item();
    match dto::wi_detail(&s.deps.sections, &conn, &item).await {
        Some(detail) => Json(detail).into_response(),
        None => (StatusCode::NOT_FOUND, "work item not found").into_response(),
    }
}
async fn wi_states(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    let (conn, item) = q.item();
    match actions::wi_states(&s.deps.sections, &conn, &item).await {
        Some(states) => Json(states).into_response(),
        None => (StatusCode::NOT_FOUND, "work item connection not found").into_response(),
    }
}
async fn wi_state(State(s): State<AppState>, Json(req): Json<actions::WiStateReq>) -> Response {
    action_response("dashboard.wi_state", actions::wi_set_state(&s.deps.sections, req).await)
}
async fn wi_comment(State(s): State<AppState>, Json(req): Json<actions::WiCommentReq>) -> Response {
    action_response("dashboard.wi_comment", actions::wi_comment(&s.deps.sections, req).await)
}
async fn wi_assignees(State(s): State<AppState>, Query(q): Query<ItemQuery>) -> Response {
    let (conn, item) = q.item();
    match actions::wi_assignees(&s.deps.sections, &conn, &item).await {
        Some(users) => Json(users).into_response(),
        None => (StatusCode::NOT_FOUND, "work item connection not found").into_response(),
    }
}
async fn wi_assignee(State(s): State<AppState>, Json(req): Json<actions::WiAssigneeReq>) -> Response {
    action_response("dashboard.wi_assignee", actions::wi_set_assignee(&s.deps.sections, req).await)
}
async fn wi_update(State(s): State<AppState>, Json(req): Json<actions::WiUpdateReq>) -> Response {
    action_response("dashboard.wi_update", actions::wi_update(&s.deps.sections, req).await)
}

async fn pipeline_detail(State(s): State<AppState>, Query(q): Query<RunQuery>) -> Response {
    match dto::pipeline_detail(&s.deps.sections, &q.conn, &ItemRef::maybe(q.repo, q.run_id)).await {
        Some(detail) => Json(detail).into_response(),
        None => (StatusCode::NOT_FOUND, "pipeline run not found").into_response(),
    }
}
async fn pipeline_logs(State(s): State<AppState>, Query(q): Query<PipelineLogsQuery>) -> Response {
    match dto::pipeline_logs(&s.deps.sections, &q.conn, &ItemRef::maybe(q.repo, q.run_id), q.job.as_deref()).await {
        Some(text) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response(),
        None => (StatusCode::NOT_FOUND, "logs not available").into_response(),
    }
}

async fn pipeline_approval(State(s): State<AppState>, Json(req): Json<actions::PipelineApprovalReq>) -> Response {
    action_response("dashboard.pipeline_approval", actions::pipeline_approval(&s.deps.sections, req).await)
}
async fn pipeline_trigger(State(s): State<AppState>, Json(req): Json<actions::PipelineTriggerReq>) -> Response {
    action_response("dashboard.pipeline_trigger", actions::pipeline_trigger(&s.deps.sections, req).await)
}
async fn pipeline_cancel(State(s): State<AppState>, Json(req): Json<actions::PipelineCancelReq>) -> Response {
    action_response("dashboard.pipeline_cancel", actions::pipeline_cancel(&s.deps.sections, req).await)
}

async fn notification_read(State(s): State<AppState>, Json(req): Json<actions::NotifReadReq>) -> Response {
    action_response("dashboard.notification_read", actions::notif_read(&s.deps.sections, req).await)
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
    match connections::save(&s.deps.config, &s.deps.sections, req).await {
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

/// The repositories a connection could fetch from — the scope picker's candidate list. Only
/// this endpoint calls provider discovery, so discovery being wrong shows an empty picker; it
/// cannot stop an already-scoped connection fetching.
async fn connection_repositories(State(s): State<AppState>, Query(q): Query<IdQuery>) -> Response {
    match s.deps.sections.discover_repositories(&q.id).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct IdQuery {
    id: String,
}

#[derive(Deserialize)]
struct ScopeReq {
    id: String,
    /// The chosen repositories. An empty list is a real choice — fetch nothing — and is stored
    /// as such; it is not the same as never having chosen.
    scope: Vec<String>,
}

async fn set_connection_scope(State(s): State<AppState>, Json(req): Json<ScopeReq>) -> Response {
    match s.deps.config.set_repo_scope(&req.id, Some(req.scope)).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

/// A pipeline connection's discovered definitions and which of them are fetched — the picker's
/// candidates. Pipelines are opt-in: nothing selected (`all` false, `selected` empty) fetches
/// nothing.
#[derive(serde::Serialize)]
struct PipelineDefinitionsResp {
    definitions: Vec<forgetop_core::domain::PipelineDefinition>,
    all: bool,
    selected: Vec<String>,
}

async fn pipeline_definitions(State(s): State<AppState>, Query(q): Query<IdQuery>) -> Response {
    let source = match s.deps.sections.pipeline_source_for(&q.id).await {
        Ok(Some(source)) => source,
        Ok(None) => return (StatusCode::NOT_FOUND, "that connection doesn't support pipelines").into_response(),
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    let definitions = match source.discover().await {
        Ok(defs) => defs,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    let cfg = s.deps.config.snapshot();
    let sub = cfg.pipelines.as_ref().and_then(|p| p.subscriptions.iter().find(|s| s.connection_id == q.id));
    Json(PipelineDefinitionsResp {
        definitions,
        all: sub.is_some_and(|s| s.auto_discover_all),
        selected: sub.map(|s| s.definition_ids.clone()).unwrap_or_default(),
    })
    .into_response()
}

#[derive(Deserialize)]
struct PipelineSelectionReq {
    id: String,
    /// Every pipeline, including ones created later. Wins over `ids`.
    #[serde(default)]
    all: bool,
    /// The chosen definitions. Empty is a real choice — fetch nothing.
    #[serde(default)]
    ids: Vec<String>,
}

async fn set_pipeline_selection(State(s): State<AppState>, Json(req): Json<PipelineSelectionReq>) -> Response {
    let saved = if req.all {
        s.deps.config.set_pipeline_auto_discover(&req.id, true).await
    } else {
        s.deps.config.set_pipeline_definitions(&req.id, req.ids).await
    };
    match saved {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    //! The optional pull-request writes (resolve a thread, draft, close/reopen, request a
    //! reviewer) end to end through the HTTP API, against the in-memory demo provider — the same
    //! harness `tests/api.rs` uses for the other routes.

    use super::*;
    use forgetop_core::config::{ConfigStore, ForgetopConfig, InMemoryConfigStore};
    use forgetop_core::domain::ProviderType;
    use forgetop_core::provider::{Connection, ProviderRegistry};
    use forgetop_core::secret::InMemorySecretStore;
    use forgetop_core::service::ConnectionResolver;
    use forgetop_providers::demo::demo_factories;

    /// One demo GitHub connection bound to the PR section — the shape `forgetop --demo` wires.
    async fn demo_deps() -> Deps {
        let registry = Arc::new(ProviderRegistry::new(demo_factories()));
        let store: Arc<dyn ConfigStore> = Arc::new(InMemoryConfigStore::new(ForgetopConfig::default()));
        let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
        let config = Arc::new(ConfigService::new(store, secrets.clone(), registry.clone()));
        config.load().await.unwrap();
        let conn = Connection {
            id: "github".into(),
            provider_type: ProviderType::GitHub,
            display_name: "github".into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        };
        config.add_or_update_connection(conn, None).await.unwrap();
        config.bind_pull_requests("github").await.unwrap();
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets.clone()));
        let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
        let health = Arc::new(ConnectionHealthService::new(config.clone(), resolver));
        Deps { sections, health, config, secrets }
    }

    struct Api {
        base: String,
        token: String,
        client: reqwest::Client,
    }

    impl Api {
        async fn start() -> Api {
            let server = spawn(demo_deps().await, 0).await.expect("server binds a free port");
            Api { base: format!("http://127.0.0.1:{}", server.port), token: server.token, client: reqwest::Client::new() }
        }
        async fn get(&self, path: &str) -> serde_json::Value {
            let r = self.client.get(format!("{}{path}", self.base)).header("x-forgetop-token", &self.token).send().await.unwrap();
            assert_eq!(r.status(), 200, "GET {path}");
            r.json().await.unwrap()
        }
        async fn post(&self, path: &str, body: serde_json::Value) -> u16 {
            let r = self.client.post(format!("{}{path}", self.base)).header("x-forgetop-token", &self.token).json(&body).send().await.unwrap();
            r.status().as_u16()
        }
        /// The open PRs the list shows, as `(id, status)`.
        async fn open_prs(&self) -> Vec<(String, String)> {
            let rows = self.get("/api/pull-requests").await;
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| (r["pull_request"]["id"].as_str().unwrap().to_string(), r["pull_request"]["status"].as_str().unwrap().to_string()))
                .collect()
        }
        async fn detail(&self, id: &str) -> serde_json::Value {
            self.get(&format!("/api/pr/detail?conn=github&id={id}")).await
        }
    }

    /// The demo PRs are global to the process, so each test picks its own open PR to write to.
    async fn open_pr(api: &Api, nth: usize) -> String {
        let open: Vec<_> = api.open_prs().await.into_iter().filter(|(_, s)| s == "Open").collect();
        open.get(nth).unwrap_or_else(|| panic!("demo has at least {} open PRs", nth + 1)).0.clone()
    }

    #[tokio::test]
    async fn pr_detail_advertises_the_demo_writes() {
        let api = Api::start().await;
        let id = open_pr(&api, 0).await;
        let writes = &api.detail(&id).await["writes"];
        for k in ["resolve_threads", "draft", "close", "reopen", "request_reviewer"] {
            assert_eq!(writes[k], true, "demo supports {k}: {writes}");
        }
    }

    #[tokio::test]
    async fn resolving_a_thread_flips_it_on_the_next_detail() {
        let api = Api::start().await;
        let id = open_pr(&api, 1).await;
        let thread = |d: &serde_json::Value, t: &str| d["threads"].as_array().unwrap().iter().find(|x| x["id"] == t).cloned().unwrap();
        assert_eq!(thread(&api.detail(&id).await, "t1")["is_resolved"], false, "t1 starts open");

        let body = |resolved: bool| serde_json::json!({ "conn": "github", "id": id, "thread_id": "t1", "resolved": resolved });
        assert_eq!(api.post("/api/pr/resolve-thread", body(true)).await, 200);
        assert_eq!(thread(&api.detail(&id).await, "t1")["is_resolved"], true, "resolve persists");
        assert_eq!(api.post("/api/pr/resolve-thread", body(false)).await, 200);
        assert_eq!(thread(&api.detail(&id).await, "t1")["is_resolved"], false, "reopen persists");
    }

    #[tokio::test]
    async fn draft_and_close_change_the_pr_status() {
        let api = Api::start().await;
        let id = open_pr(&api, 2).await;
        let status = |d: serde_json::Value| d["pull_request"]["status"].as_str().unwrap().to_string();

        assert_eq!(api.post("/api/pr/draft", serde_json::json!({ "conn": "github", "id": id, "draft": true })).await, 200);
        assert_eq!(status(api.detail(&id).await), "Draft", "converted to a draft");
        assert_eq!(api.post("/api/pr/draft", serde_json::json!({ "conn": "github", "id": id, "draft": false })).await, 200);
        assert_eq!(status(api.detail(&id).await), "Open", "ready for review again");

        assert_eq!(api.post("/api/pr/closed", serde_json::json!({ "conn": "github", "id": id, "closed": true })).await, 200);
        assert_eq!(status(api.detail(&id).await), "Closed");
        assert!(!api.open_prs().await.iter().any(|(p, _)| *p == id), "a closed PR leaves the open list");
        assert_eq!(api.post("/api/pr/closed", serde_json::json!({ "conn": "github", "id": id, "closed": false })).await, 200);
        assert_eq!(status(api.detail(&id).await), "Open", "reopened");
        assert!(api.open_prs().await.iter().any(|(p, _)| *p == id), "and back on the open list");

        // A missing connection is a 404, as for every other action.
        assert_eq!(api.post("/api/pr/draft", serde_json::json!({ "conn": "nope", "id": id, "draft": true })).await, 404);
    }

    #[tokio::test]
    async fn requesting_a_reviewer_adds_them_to_the_pr() {
        let api = Api::start().await;
        let id = open_pr(&api, 3).await;
        let users = api.get(&format!("/api/pr/reviewers?conn=github&id={id}")).await;
        let users = users.as_array().expect("reviewers is a JSON array of users");
        assert!(!users.is_empty(), "the demo has someone to ask");

        let reviewer_ids = |d: &serde_json::Value| -> Vec<String> {
            d["pull_request"]["reviewers"].as_array().unwrap().iter().map(|r| r["user"]["id"].as_str().unwrap().to_string()).collect()
        };
        let before = reviewer_ids(&api.detail(&id).await);
        let pick = users.iter().map(|u| u["id"].as_str().unwrap().to_string()).find(|u| !before.contains(u)).expect("someone not yet reviewing");

        assert_eq!(api.post("/api/pr/request-reviewer", serde_json::json!({ "conn": "github", "id": id, "user_id": pick })).await, 200);
        let after = api.detail(&id).await;
        let added = after["pull_request"]["reviewers"].as_array().unwrap().iter().find(|r| r["user"]["id"] == pick.as_str()).cloned();
        let added = added.expect("the requested reviewer is on the PR");
        assert_eq!(added["vote"], "NoVote", "requested, not yet voted");

        let missing = api.client.get(format!("{}/api/pr/reviewers?conn=nope&id={id}", api.base)).header("x-forgetop-token", &api.token).send().await.unwrap();
        assert_eq!(missing.status(), 404, "a bad connection is a 404");
    }
}
