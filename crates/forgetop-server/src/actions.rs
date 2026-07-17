//! Write actions for the dashboard. Each resolves the capability-scoped source for a connection
//! and calls through to the provider — the same calls the TUI makes. All of these sit behind the
//! session-token auth layer (see `lib.rs`); the server binds localhost-only, so a merge or a
//! state change can't be triggered by another origin.

use std::sync::Arc;

use forgetop_core::domain::{ApprovalDecision, LineComment, ReviewVote};
use forgetop_core::provider::{MergeOptions, MergeStrategy, NotificationSource};
use forgetop_core::service::SectionService;
use serde::Deserialize;

use crate::dto;

/// Why an action couldn't be performed: the connection/capability wasn't found (→ 404), or the
/// provider call itself failed (→ 502).
pub enum ActionError {
    NotFound,
    Failed(String),
}

fn failed(e: impl std::fmt::Display) -> ActionError {
    ActionError::Failed(e.to_string())
}

// ---- source resolvers ----
//
// Work-item and pipeline resolvers live in `dto` (shared with the read/detail endpoints);
// only the notification resolver is action-only.

async fn notif_source(sections: &SectionService, conn: &str) -> Option<Arc<dyn NotificationSource>> {
    sections.notification_feeds().await.ok()?.into_iter().find(|f| f.connection.connection_id() == conn).map(|f| f.source)
}

// ---- request bodies ----

#[derive(Deserialize)]
pub struct PrVoteReq {
    pub conn: String,
    pub id: String,
    pub vote: ReviewVote,
}

#[derive(Deserialize)]
pub struct PrMergeReq {
    pub conn: String,
    pub id: String,
    #[serde(default)]
    pub strategy: MergeStrategy,
    #[serde(default)]
    pub delete_source_ref: bool,
}

#[derive(Deserialize)]
pub struct PrCommentReq {
    pub conn: String,
    pub id: String,
    pub body: String,
}

#[derive(Deserialize)]
pub struct PrReviewReq {
    pub conn: String,
    pub id: String,
    pub event: ReviewVote,
    #[serde(default)]
    pub comments: Vec<LineComment>,
}

#[derive(Deserialize)]
pub struct WiStateReq {
    pub conn: String,
    pub id: String,
    pub state: String,
}

#[derive(Deserialize)]
pub struct WiCommentReq {
    pub conn: String,
    pub id: String,
    pub body: String,
}

#[derive(Deserialize)]
pub struct PipelineApprovalReq {
    pub conn: String,
    pub run_id: String,
    pub approval_id: String,
    pub decision: ApprovalDecision,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Deserialize)]
pub struct PipelineTriggerReq {
    pub conn: String,
    pub definition_id: String,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Deserialize)]
pub struct NotifReadReq {
    pub conn: String,
    pub id: String,
}

// ---- actions ----

pub async fn pr_vote(sections: &SectionService, req: PrVoteReq) -> Result<(), ActionError> {
    let source = dto::pr_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.vote(&req.id, req.vote).await.map_err(failed)
}

pub async fn pr_merge(sections: &SectionService, req: PrMergeReq) -> Result<(), ActionError> {
    let source = dto::pr_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    let options = MergeOptions { strategy: req.strategy, delete_source_ref: req.delete_source_ref };
    source.merge(&req.id, &options).await.map_err(failed)
}

pub async fn pr_comment(sections: &SectionService, req: PrCommentReq) -> Result<(), ActionError> {
    let source = dto::pr_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.add_comment(&req.id, &req.body).await.map_err(failed)
}

pub async fn pr_review(sections: &SectionService, req: PrReviewReq) -> Result<(), ActionError> {
    let source = dto::pr_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.submit_review(&req.id, req.event, &req.comments).await.map_err(failed)
}

/// The states a work item can move to (for the transition menu).
pub async fn wi_states(sections: &SectionService, conn: &str, id: &str) -> Option<Vec<String>> {
    let source = dto::wi_source(sections, conn).await?;
    Some(source.available_states(id).await.unwrap_or_default())
}

pub async fn wi_set_state(sections: &SectionService, req: WiStateReq) -> Result<(), ActionError> {
    let source = dto::wi_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.set_state(&req.id, &req.state).await.map_err(failed)
}

pub async fn wi_comment(sections: &SectionService, req: WiCommentReq) -> Result<(), ActionError> {
    let source = dto::wi_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.add_comment(&req.id, &req.body).await.map_err(failed)
}

pub async fn pipeline_approval(sections: &SectionService, req: PipelineApprovalReq) -> Result<(), ActionError> {
    let source = dto::pipe_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.respond_approval(&req.run_id, &req.approval_id, req.decision, req.comment.as_deref()).await.map_err(failed)
}

pub async fn pipeline_trigger(sections: &SectionService, req: PipelineTriggerReq) -> Result<(), ActionError> {
    let source = dto::pipe_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.trigger(&req.definition_id, req.branch.as_deref()).await.map_err(failed)
}

pub async fn notif_read(sections: &SectionService, req: NotifReadReq) -> Result<(), ActionError> {
    let source = notif_source(sections, &req.conn).await.ok_or(ActionError::NotFound)?;
    source.mark_read(&req.id).await.map_err(failed)
}
