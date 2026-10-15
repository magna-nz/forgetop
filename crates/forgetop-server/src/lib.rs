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
use forgetop_core::cache::CacheStore;
use forgetop_core::domain::{Notification, NotificationDetail};
use forgetop_core::launchpad::{Entry, EntryKind};
use forgetop_core::provider::{ItemRef, PrDecoration};
use forgetop_core::secret::SecretStore;
use forgetop_core::service::{ConfigService, ConnectionHealthService, SectionService};
use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::actions::ActionError;

pub mod cached;

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
    /// The on-disk cache the TUI paints from at launch, shared so the dashboard does too: lists
    /// answer from it first and the live fetch writes back (see `cached`). Pass a disabled store
    /// to cache nothing (`--demo`).
    pub cache: Arc<CacheStore>,
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

/// Notification details (actor, excerpt) fetched on demand, per `(connection, id, detail_ref)` —
/// so reopening a notification costs no call (the GitHub budget), while new activity on the same
/// thread (a new latest comment, so a new `detail_ref`) is fetched afresh.
#[derive(Default)]
struct NotifDetailCache {
    entries: std::sync::Mutex<std::collections::HashMap<String, NotificationDetail>>,
}

impl NotifDetailCache {
    /// Bounded like [`DecorationCache`], and dropped whole on overflow for the same reason.
    const MAX: usize = 500;

    fn key(conn: &str, n: &Notification) -> String {
        format!("{conn}\u{1}{}\u{1}{}", n.id, n.detail_ref.as_deref().unwrap_or(""))
    }

    fn get(&self, key: &str) -> Option<NotificationDetail> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).get(key).cloned()
    }

    fn put(&self, key: String, value: NotificationDetail) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
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
    notif_details: Arc<NotifDetailCache>,
    /// Lists with a live fetch out right now, by key: a poll that lands meanwhile answers from
    /// the cache rather than starting another.
    inflight: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    flusher: Arc<cached::Flusher>,
}

async fn bind(deps: Deps, port: u16) -> std::io::Result<(tokio::net::TcpListener, Server, AppState)> {
    let token = uuid::Uuid::new_v4().to_string();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let bound = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{bound}/?t={token}");
    let flusher = Arc::new(cached::Flusher::new(deps.cache.clone()));
    let state = AppState {
        deps,
        token: Arc::from(token.as_str()),
        decorations: Arc::new(DecorationCache::default()),
        notif_details: Arc::new(NotifDetailCache::default()),
        inflight: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        flusher,
    };
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
        .route("/api/launchpad/dismiss", post(dismiss_launchpad_item))
        .route("/api/prefs", get(prefs))
        .route("/api/prefs/hidden-work-item-state", post(set_hidden_work_item_state))
        .route("/api/pr/detail", get(pr_detail))
        .route("/api/pr/decoration", get(pr_decoration))
        .route("/api/pr/commit-changes", get(pr_commit_changes))
        .route("/api/pr/file-text", get(pr_file_text))
        .route("/api/highlight", post(highlight))
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
        .route("/api/pipeline/annotations", get(pipeline_annotations))
        .route("/api/pipeline/artifacts", get(pipeline_artifacts))
        .route("/api/pipeline/approval", post(pipeline_approval))
        .route("/api/pipeline/trigger", post(pipeline_trigger))
        .route("/api/pipeline/cancel", post(pipeline_cancel))
        .route("/api/pipeline/rerun", post(pipeline_rerun))
        .route("/api/notification/read", post(notification_read))
        .route("/api/notification/read-many", post(notification_read_many))
        .route("/api/notification/done", post(notification_done))
        .route("/api/notification/detail", post(notification_detail))
        .route("/api/inbox", get(inbox))
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
/// The response header naming when a cached answer's data was fetched. Absent on a live answer.
pub const FETCHED_AT_HEADER: &str = "x-forgetop-fetched-at";

/// How long a list request waits for its live fetch before answering from the shared cache
/// instead (the fetch carries on behind the answer and writes back). Long enough that a quick
/// forge still answers live on every poll, short enough that a slow one never leaves the page on
/// skeletons. `FORGETOP_CACHE_WAIT_MS` overrides it; 0 always answers from the cache when it can.
const STALE_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

fn stale_wait() -> std::time::Duration {
    std::env::var("FORGETOP_CACHE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(STALE_WAIT)
}

/// Answers a list live when the forge is quick, and from the shared cache when it is not: the
/// live fetch starts at once, the request waits `stale_wait()` for it, and past that the cached
/// rows go out (dated by the header) while the fetch finishes behind them and writes back. With
/// nothing cached the request waits for the fetch however long it takes. One live fetch per key
/// at a time: a poll that lands while one is out answers from the cache.
async fn serve_cached<T, F, Fut>(s: &AppState, key: &str, cached: Option<cached::Cached<T>>, fetch: F) -> Response
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce(AppState) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    let claim = |s: &AppState| s.inflight.lock().unwrap_or_else(|p| p.into_inner()).insert(key.to_string());
    let from_cache = |hit: cached::Cached<T>| {
        let mut res = Json(hit.value).into_response();
        if let Ok(v) = hit.fetched_at.to_rfc3339().parse() {
            res.headers_mut().insert(FETCHED_AT_HEADER, v);
        }
        res
    };
    if !claim(s) {
        // A fetch is already out for this list; it lands shortly and the next poll sees it.
        return match cached {
            Some(hit) => from_cache(hit),
            None => Json(fetch(s.clone()).await).into_response(),
        };
    }
    let live = {
        let s2 = s.clone();
        let key = key.to_string();
        tokio::spawn(async move {
            let value = fetch(s2.clone()).await;
            s2.inflight.lock().unwrap_or_else(|p| p.into_inner()).remove(&key);
            s2.flusher.schedule();
            value
        })
    };
    let Some(hit) = cached else {
        return match live.await {
            Ok(value) => Json(value).into_response(),
            Err(_) => (StatusCode::BAD_GATEWAY, "the fetch behind this list failed").into_response(),
        };
    };
    match tokio::time::timeout(stale_wait(), live).await {
        Ok(Ok(value)) => Json(value).into_response(),
        // Slow forge, or a panicked fetch: the last known rows, dated, while the fetch runs on.
        _ => from_cache(hit),
    }
}

async fn pull_requests(State(s): State<AppState>, Query(q): Query<PrListQuery>) -> Response {
    let view = dto::PrView::parse(q.view.as_deref());
    let hit = cached::pull_requests(&s.deps.cache, view);
    serve_cached(&s, &format!("prs.{view:?}"), hit, move |s: AppState| async move { dto::pull_requests(&s.deps.sections, &s.deps.cache, view).await }).await
}
async fn work_items(State(s): State<AppState>) -> Response {
    let hit = cached::work_items(&s.deps.cache);
    serve_cached(&s, "work_items", hit, |s: AppState| async move { dto::work_items(&s.deps.sections, &s.deps.cache).await }).await
}
async fn pipelines(State(s): State<AppState>) -> Response {
    // Read-only against the cache: the launchpad fetch writes the pipelines entry, with the
    // "yours" flag the list alone cannot tell.
    let hit = cached::pipelines(&s.deps.cache);
    serve_cached(&s, "pipelines", hit, |s: AppState| async move { dto::pipelines(&s.deps.sections).await }).await
}
/// The notifications either frontend marked done (`ui.dismissed_notifications`), read once per
/// request so the cached answer and the live fetch hide the same rows.
fn dismissed_notifications(s: &AppState) -> Arc<std::collections::HashSet<String>> {
    Arc::new(s.deps.config.snapshot().ui.dismissed_notifications.into_iter().collect())
}

/// What the inbox shows of `rows`: dismissed ones dropped, and any detail already fetched for a
/// row (`/api/notification/detail`) filled in, so its phrase names the actor from then on.
fn visible_notifications(s: &AppState, rows: Vec<dto::NotifRow>, dismissed: &std::collections::HashSet<String>) -> Vec<dto::NotifRow> {
    rows.into_iter()
        .filter(|r| !dismissed.contains(&r.dismiss_key))
        .map(|r| {
            let n = &r.notification;
            if n.detail_ref.is_none() || (n.actor.is_some() && n.excerpt.is_some()) {
                return r;
            }
            match s.notif_details.get(&NotifDetailCache::key(&r.connection_id, n)) {
                Some(d) => r.with_detail(&d),
                None => r,
            }
        })
        .collect()
}

/// Both inbox endpoints share one live fetch: while one is out, the other answers from the cache.
const NOTIFICATIONS_FETCH: &str = "notifications";

async fn notifications(State(s): State<AppState>) -> Response {
    let dismissed = dismissed_notifications(&s);
    let hit = cached::notifications(&s.deps.cache)
        .map(|c| cached::Cached { value: visible_notifications(&s, c.value, &dismissed), fetched_at: c.fetched_at });
    serve_cached(&s, NOTIFICATIONS_FETCH, hit, move |s: AppState| async move {
        let rows = dto::notifications(&s.deps.sections, &s.deps.cache).await;
        visible_notifications(&s, rows, &dismissed)
    })
    .await
}

/// The notifications folded into one row per item (`forgetop_core::inbox::threads`), over the
/// same rows `/api/notifications` serves.
async fn inbox(State(s): State<AppState>) -> Response {
    let dismissed = dismissed_notifications(&s);
    let hit = cached::notifications(&s.deps.cache).map(|c| cached::Cached {
        value: dto::inbox_threads(&visible_notifications(&s, c.value, &dismissed)),
        fetched_at: c.fetched_at,
    });
    serve_cached(&s, NOTIFICATIONS_FETCH, hit, move |s: AppState| async move {
        let rows = dto::notifications(&s.deps.sections, &s.deps.cache).await;
        dto::inbox_threads(&visible_notifications(&s, rows, &dismissed))
    })
    .await
}
async fn launchpad(State(s): State<AppState>) -> Response {
    // Read once, so the cached answer and the live fetch hide the same entries.
    let dismissed = Arc::new(s.deps.config.snapshot().ui.dismissed_launchpad_items);
    let hit = cached::launchpad(&s.deps.cache, &dismissed);
    serve_cached(&s, "launchpad", hit, move |s: AppState| async move { dto::launchpad(&s.deps.sections, &s.deps.cache, &dismissed).await }).await
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

/// Query params naming one file of a PR (`?conn=…&id=…&path=…`), as of a commit (`&sha=…`) or,
/// without one, the PR's head.
#[derive(Deserialize)]
struct FileTextQuery {
    conn: String,
    id: String,
    path: String,
    #[serde(default)]
    sha: Option<String>,
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
    action_response_with(operation, result.map(|()| serde_json::json!({ "ok": true })))
}

/// Like [`action_response`], for an action that reports something back: `Ok` carries the JSON body.
fn action_response_with(operation: &'static str, result: Result<serde_json::Value, ActionError>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
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

/// One file's full text, so a diff can expand the unchanged lines between its hunks — plain text,
/// or a 404 when there's no such connection, or the forge has no text for it (binary, oversized,
/// missing at that commit, or a provider without a content API).
async fn pr_file_text(State(s): State<AppState>, Query(q): Query<FileTextQuery>) -> Response {
    match dto::pr_file_text(&s.deps.sections, &q.conn, &ItemRef::maybe(q.repo, q.id), &q.path, q.sha.as_deref()).await {
        Some(text) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response(),
        None => (StatusCode::NOT_FOUND, "file text not available").into_response(),
    }
}

/// Source lines to syntax-highlight, and the path whose extension picks the language.
#[derive(Deserialize)]
struct HighlightReq {
    path: String,
    lines: Vec<String>,
}

/// `POST /api/highlight`: the dashboard sends the lines of a diff (or of a file whose gaps it
/// is expanding) and gets back per-line `[len, kind]` spans — see [`dto::Highlight`]. Pure
/// CPU over the request body, no provider involved; the same grammars the TUI uses.
async fn highlight(Json(req): Json<HighlightReq>) -> Json<dto::Highlight> {
    Json(dto::highlight(&req.path, &req.lines))
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

async fn pipeline_annotations(State(s): State<AppState>, Query(q): Query<RunQuery>) -> Response {
    match dto::pipeline_annotations(&s.deps.sections, &q.conn, &ItemRef::maybe(q.repo, q.run_id)).await {
        Some(annotations) => Json(annotations).into_response(),
        None => (StatusCode::NOT_FOUND, "annotations not available").into_response(),
    }
}
async fn pipeline_artifacts(State(s): State<AppState>, Query(q): Query<RunQuery>) -> Response {
    match dto::pipeline_artifacts(&s.deps.sections, &q.conn, &ItemRef::maybe(q.repo, q.run_id)).await {
        Some(artifacts) => Json(artifacts).into_response(),
        None => (StatusCode::NOT_FOUND, "artifacts not available").into_response(),
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
async fn pipeline_rerun(State(s): State<AppState>, Json(req): Json<actions::PipelineRerunReq>) -> Response {
    let result = actions::pipeline_rerun(&s.deps.sections, req).await;
    action_response_with("dashboard.pipeline_rerun", result.map(|new_run_id| serde_json::json!({ "ok": true, "new_run_id": new_run_id })))
}

async fn notification_read(State(s): State<AppState>, Json(req): Json<actions::NotifReadReq>) -> Response {
    action_response("dashboard.notification_read", actions::notif_read(&s.deps.sections, req).await)
}

/// The notifications an action resolves `ids` against — every row, dismissed ones included. The
/// shared cache answers when it knows every id (the inbox was just polled); otherwise one live
/// fetch, which writes the cache back like any other.
async fn current_notifications(s: &AppState, conn: &str, ids: &[String]) -> Vec<dto::NotifRow> {
    if let Some(hit) = cached::notifications(&s.deps.cache) {
        let knows = |id: &String| hit.value.iter().any(|r| r.connection_id == conn && r.notification.id == *id);
        if ids.iter().all(knows) {
            return hit.value;
        }
    }
    let rows = dto::notifications(&s.deps.sections, &s.deps.cache).await;
    s.flusher.schedule();
    rows
}

async fn notification_read_many(State(s): State<AppState>, Json(req): Json<actions::NotifIdsReq>) -> Response {
    let rows = current_notifications(&s, &req.conn, &req.ids).await;
    action_response("dashboard.notification_read_many", actions::notif_read_many(&s.deps.sections, &rows, &req).await)
}

/// "Done" on an inbox thread: read at the forge, then dismissed under the TUI's key.
async fn notification_done(State(s): State<AppState>, Json(req): Json<actions::NotifIdsReq>) -> Response {
    let rows = current_notifications(&s, &req.conn, &req.ids).await;
    action_response("dashboard.notification_done", actions::notif_done(&s.deps.sections, &s.deps.config, &rows, &req).await)
}

#[derive(Deserialize)]
struct NotifDetailReq {
    conn: String,
    id: String,
}

/// The actor and excerpt of one notification, fetched once per `(connection, id, detail_ref)` and
/// kept for the session. A notification whose feed entry already carries them answers with those,
/// without a call.
async fn notification_detail(State(s): State<AppState>, Json(req): Json<NotifDetailReq>) -> Response {
    let rows = current_notifications(&s, &req.conn, std::slice::from_ref(&req.id)).await;
    let Some(row) = rows.into_iter().find(|r| r.connection_id == req.conn && r.notification.id == req.id) else {
        return (StatusCode::NOT_FOUND, "notification not found").into_response();
    };
    let detail = if row.notification.detail_ref.is_none() {
        // Nothing to fetch: the feed already said all it can.
        NotificationDetail::default()
    } else {
        let key = NotifDetailCache::key(&req.conn, &row.notification);
        match s.notif_details.get(&key) {
            Some(hit) => hit,
            None => match actions::notif_detail(&s.deps.sections, &req.conn, &row.notification).await {
                Ok(d) => {
                    s.notif_details.put(key, d.clone());
                    d
                }
                Err(e) => {
                    // Remembered as "nothing more", so a deleted comment isn't asked for again on
                    // every open: each ask is a call against the hourly budget.
                    s.notif_details.put(key, NotificationDetail::default());
                    return action_response_with("dashboard.notification_detail", Err(e));
                }
            },
        }
    };
    let target = detail.target.clone();
    // What the feed carried wins over a fetched guess; the phrase and why follow the filled row.
    let filled = row.with_detail(&detail);
    Json(serde_json::json!({
        "actor": filled.notification.actor,
        "excerpt": filled.notification.excerpt,
        "target": target,
        "url": filled.notification.url,
        "phrase": filled.phrase,
        "why": filled.why,
    }))
    .into_response()
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

#[derive(Deserialize)]
struct DismissLaunchpadReq {
    conn: String,
    /// `"pr"`, `"wi"` or `"pipe"` — the tag of a `launchpad::EntryKind`.
    kind: String,
    #[serde(default)]
    repo: Option<String>,
    id: String,
}

/// Permanently dismisses one Command Center item, under the same key the TUI's `D` writes.
async fn dismiss_launchpad_item(State(s): State<AppState>, Json(req): Json<DismissLaunchpadReq>) -> Response {
    let kind = match req.kind.as_str() {
        "pr" => EntryKind::Pr,
        "wi" => EntryKind::Wi,
        "pipe" => EntryKind::Pipe,
        other => return (StatusCode::BAD_REQUEST, format!("unknown launchpad kind `{other}`")).into_response(),
    };
    let key = Entry::key(&req.conn, kind, req.repo.as_deref(), &req.id);
    // Merged into the saved list, not written over it: the TUI adds to the same list.
    match s.deps.config.dismiss_launchpad_item(key).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

/// View preferences the dashboard shares with the TUI.
async fn prefs(State(s): State<AppState>) -> Response {
    let ui = s.deps.config.snapshot().ui;
    Json(serde_json::json!({ "hidden_work_item_states": ui.hidden_work_item_states })).into_response()
}

#[derive(Deserialize)]
struct HiddenWorkItemStateReq {
    state: String,
    hidden: bool,
}

/// Hides or shows one work-item state. One state at a time, merged into the saved list, so the
/// dashboard never writes over what the TUI's "Show states" picker has set meanwhile.
async fn set_hidden_work_item_state(State(s): State<AppState>, Json(req): Json<HiddenWorkItemStateReq>) -> Response {
    match s.deps.config.set_hidden_work_item_state(&req.state, req.hidden).await {
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
        deps_with(Arc::new(CacheStore::disabled())).await
    }

    async fn deps_with(cache: Arc<CacheStore>) -> Deps {
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
        config.bind_work_items("github").await.unwrap();
        config.set_pipeline_auto_discover("github", true).await.unwrap();
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets.clone()));
        let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
        let health = Arc::new(ConnectionHealthService::new(config.clone(), resolver));
        Deps { sections, health, config, secrets, cache }
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

        async fn start_with(deps: Deps) -> Api {
            let server = spawn(deps, 0).await.expect("server binds a free port");
            Api { base: format!("http://127.0.0.1:{}", server.port), token: server.token, client: reqwest::Client::new() }
        }

        async fn get_raw(&self, path: &str) -> reqwest::Response {
            self.client.get(format!("{}{path}", self.base)).header("x-forgetop-token", &self.token).send().await.expect("request")
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
    /// The GitHub connection has exactly four open ones, and `draft_and_close` takes one out of
    /// the open list for a moment, so the PR tests run one at a time: a listing must never land
    /// inside that window.
    static DEMO_PRS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn open_pr(api: &Api, nth: usize) -> String {
        let open: Vec<_> = api.open_prs().await.into_iter().filter(|(_, s)| s == "Open").collect();
        open.get(nth).unwrap_or_else(|| panic!("demo has at least {} open PRs", nth + 1)).0.clone()
    }

    #[tokio::test]
    async fn pr_detail_advertises_the_demo_writes() {
        let _one_at_a_time = DEMO_PRS.lock().await;
        let api = Api::start().await;
        let id = open_pr(&api, 0).await;
        let writes = &api.detail(&id).await["writes"];
        for k in ["resolve_threads", "draft", "close", "reopen", "request_reviewer"] {
            assert_eq!(writes[k], true, "demo supports {k}: {writes}");
        }
    }

    #[tokio::test]
    async fn resolving_a_thread_flips_it_on_the_next_detail() {
        let _one_at_a_time = DEMO_PRS.lock().await;
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
        let _one_at_a_time = DEMO_PRS.lock().await;
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
        let _one_at_a_time = DEMO_PRS.lock().await;
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

    fn scratch_cache() -> (Arc<CacheStore>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("forgetop-api-cache-{}.json", uuid::Uuid::new_v4()));
        (Arc::new(CacheStore::new(&path)), path)
    }

    /// A list answers from the shared cache at once, says so in a header, and the live fetch
    /// behind it replaces the entry.
    #[tokio::test]
    async fn lists_answer_from_the_shared_cache_then_refresh_behind_it() {
        // Demo data answers in microseconds, so to see the cached path the wait is turned off.
        std::env::set_var("FORGETOP_CACHE_WAIT_MS", "0");
        let (cache, path) = scratch_cache();
        let api = Api::start_with(deps_with(cache.clone()).await).await;

        // Cold: the live fetch answers and writes the TUI's entry.
        let cold = api.get_raw("/api/work-items").await;
        assert!(cold.headers().get(FETCHED_AT_HEADER).is_none(), "a live answer carries no cache date");
        let live: Vec<serde_json::Value> = cold.json().await.unwrap();
        assert!(!live.is_empty());
        assert!(cache.get::<Vec<cached::TuiWiRow>>(cached::KEY_WORK_ITEMS).is_some(), "the live fetch wrote the TUI's key");

        // Make the cached entry recognisably stale, as a TUI that ran earlier would have left it.
        assert!(cache.rewrite::<Vec<cached::TuiWiRow>>(cached::KEY_WORK_ITEMS, |mut rows| {
            rows[0].wi.title = "Left here by the terminal".into();
            rows
        }));
        let warm = api.get_raw("/api/work-items").await;
        let fetched_at = warm.headers().get(FETCHED_AT_HEADER).expect("a cached answer says when it was fetched").clone();
        assert!(chrono::DateTime::parse_from_rfc3339(fetched_at.to_str().unwrap()).is_ok());
        let rows: Vec<serde_json::Value> = warm.json().await.unwrap();
        assert_eq!(rows[0]["work_item"]["title"], "Left here by the terminal");

        // The fetch that ran behind that answer has replaced the entry by now (demo data is instant).
        let mut fresh = Vec::new();
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            fresh = api.get_raw("/api/work-items").await.json::<Vec<serde_json::Value>>().await.unwrap();
            if fresh[0]["work_item"]["title"] != "Left here by the terminal" {
                break;
            }
        }
        assert_ne!(fresh[0]["work_item"]["title"], "Left here by the terminal", "the live fetch wrote back");
        std::env::remove_var("FORGETOP_CACHE_WAIT_MS");
        // And the flusher put it on disk for the next process.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert!(path.exists(), "the cache was flushed to {}", path.display());
        let _ = std::fs::remove_file(path);
    }

    /// A dismissal hides the entry on the cached path too, by the current key or the legacy one.
    #[tokio::test]
    async fn the_cached_launchpad_drops_dismissed_entries() {
        let (cache, path) = scratch_cache();
        let api = Api::start_with(deps_with(cache.clone()).await).await;
        let _ = api.get_raw("/api/launchpad").await;
        let all = cached::launchpad(&cache, &[]).expect("launchpad from cache").value;
        let first = serde_json::to_value(&all.rows[0]).unwrap();
        let (conn, id) = (first["connection_id"].as_str().unwrap().to_string(), first[match first["kind"].as_str().unwrap() {
            "pr" => "pull_request",
            "wi" => "work_item",
            _ => "run",
        }]["id"]
            .as_str()
            .unwrap()
            .to_string());
        let legacy = cached::launchpad(&cache, &[format!("{conn}:{id}")]).expect("launchpad from cache").value;
        assert!(legacy.rows.len() < all.rows.len(), "the legacy key hides the entry");
        // The current key (kind and repository included) hides it too.
        let kind = first["kind"].as_str().unwrap();
        let item = &first[match kind {
            "pr" => "pull_request",
            "wi" => "work_item",
            _ => "run",
        }];
        let repo = item["repository"].as_str().unwrap_or("");
        let current = cached::launchpad(&cache, &[format!("{conn}:{kind}:{repo}:{id}")]).expect("launchpad from cache").value;
        assert_eq!(current.rows.len(), legacy.rows.len(), "the current key hides the same entry");
        // A key that matches nothing hides nothing.
        let other = cached::launchpad(&cache, &["nope:pr::0".to_string()]).expect("launchpad from cache").value;
        assert_eq!(other.rows.len(), all.rows.len());
        let _ = std::fs::remove_file(path);
    }

    /// Every list the dashboard fetches lands in the cache under the key the TUI reads at launch.
    #[tokio::test]
    async fn live_fetches_write_the_terminal_ui_keys() {
        let (cache, path) = scratch_cache();
        let api = Api::start_with(deps_with(cache.clone()).await).await;
        let _ = api.get_raw("/api/pull-requests?view=all").await;
        let _ = api.get_raw("/api/pull-requests?view=review_requested").await;
        let _ = api.get_raw("/api/launchpad").await;
        let _ = api.get_raw("/api/notifications").await;
        let all = cache.get::<Vec<cached::TuiPrRow>>(&cached::prs_key(forgetop_core::provider::PullRequestFilter::All, false)).expect("All view");
        assert!(!all.value.is_empty());
        assert!(cache.get::<Vec<cached::TuiPrRow>>(&cached::prs_key(forgetop_core::provider::PullRequestFilter::ReviewRequested, false)).is_some());
        assert!(cache.get::<Vec<cached::TuiPrRow>>(cached::KEY_LAUNCHPAD_MINE).is_some());
        assert!(cache.get::<Vec<cached::TuiPrRow>>(cached::KEY_LAUNCHPAD_REVIEW).is_some());
        assert!(cache.get::<Vec<cached::TuiWiRow>>(cached::KEY_WORK_ITEMS).is_some());
        assert!(cache.get::<Vec<cached::TuiPipeRow>>(cached::KEY_PIPELINES).is_some());
        assert!(cache.get::<Vec<cached::TuiNotifRow>>(cached::KEY_INBOX).is_some());
        // The Command Center rebuilt from those entries is the one the live build produced.
        let live: serde_json::Value = api.get_raw("/api/launchpad").await.json().await.unwrap();
        let from_cache = cached::launchpad(&cache, &[]).expect("launchpad from cache");
        assert_eq!(serde_json::to_value(&from_cache.value).unwrap(), live);
        let _ = std::fs::remove_file(path);
    }
}
