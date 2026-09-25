import { demoFixture } from "./demoFixtures";
import type { DemoAction, DemoComment, DemoPerson, DemoState, DemoTimelineEvent, WorkItemCategory } from "./demoTypes";

let nextId = 1;

function clone<T>(value: T): T {
  return structuredClone(value);
}

function actionTime(): string {
  return "Just now";
}

function localId(prefix: string): string {
  const id = `${prefix}-${nextId}`;
  nextId += 1;
  return id;
}

function comment(author: DemoPerson, body: string, replyTo?: string): DemoComment {
  return { id: localId("comment"), author, body: body.trim(), createdAt: actionTime(), ...(replyTo ? { replyTo } : {}) };
}

function event(actor: DemoPerson, kind: DemoTimelineEvent["kind"], message: string): DemoTimelineEvent {
  return { id: localId("event"), actor, kind, message, at: actionTime() };
}

function stateName(category: WorkItemCategory): string {
  return ({ backlog: "Backlog", todo: "Todo", in_progress: "In Progress", done: "Done", blocked: "Blocked" })[category];
}

/** Returns an entirely fresh browser-memory state. Refreshing the page invokes this again. */
export function createDemoState(): DemoState {
  nextId = 1;
  return clone(demoFixture);
}

function withLastAction(state: DemoState, lastAction: string): DemoState {
  return { ...state, lastAction };
}

/**
 * The public demo's complete mutation surface. It is a pure reducer; callers
 * own the state and no action can leave this browser process.
 */
export function reduceDemo(state: DemoState, action: DemoAction): DemoState {
  if (action.type === "reset") return createDemoState();
  const next = clone(state);
  const me = next.currentUser;

  switch (action.type) {
    case "pr.comment": {
      const pr = next.pullRequests.find((item) => item.id === action.prId);
      if (!pr || !action.body.trim()) return state;
      pr.comments.push(comment(me, action.body, action.replyTo));
      pr.timeline.unshift(event(me, "comment", action.replyTo ? "replied to a review comment" : "commented on this pull request"));
      return withLastAction(next, "Comment posted (simulated)");
    }
    case "pr.review": {
      const pr = next.pullRequests.find((item) => item.id === action.prId);
      if (!pr || pr.status !== "open") return state;
      const review = pr.reviewers.find((item) => item.reviewer.id === me.id);
      const message = action.vote === "approved" ? "approved these changes" : "requested changes";
      if (review) Object.assign(review, { vote: action.vote, summary: action.summary, at: actionTime() });
      else pr.reviewers.push({ reviewer: me, vote: action.vote, summary: action.summary, at: actionTime() });
      pr.timeline.unshift(event(me, "review", message));
      return withLastAction(next, action.vote === "approved" ? "Review approved (simulated)" : "Changes requested (simulated)");
    }
    case "pr.merge": {
      const pr = next.pullRequests.find((item) => item.id === action.prId);
      if (!pr || pr.status !== "open" || !pr.mergeable || pr.checks !== "passing") return state;
      pr.status = "merged";
      pr.mergeable = false;
      pr.timeline.unshift(event(me, "merge", "merged this pull request"));
      next.notifications = next.notifications.map((notice) => notice.targetId === pr.id ? { ...notice, unread: false } : notice);
      return withLastAction(next, "Pull request merged (simulated)");
    }
    case "pr.revert": {
      const pr = next.pullRequests.find((item) => item.id === action.prId);
      if (!pr || pr.status !== "merged") return state;
      pr.timeline.unshift(event(me, "revert", "created simulated revert feedback"));
      return withLastAction(next, "Revert prepared (simulated)");
    }
    case "work-item.comment": {
      const item = next.workItems.find((entry) => entry.id === action.workItemId);
      if (!item || !action.body.trim()) return state;
      item.comments.push(comment(me, action.body, action.replyTo));
      item.timeline.unshift(event(me, "comment", "commented on this work item"));
      return withLastAction(next, "Comment posted (simulated)");
    }
    case "work-item.assign": {
      const item = next.workItems.find((entry) => entry.id === action.workItemId);
      if (!item) return state;
      item.assignee = action.assigneeId ? next.people.find((person) => person.id === action.assigneeId) ?? null : null;
      item.timeline.unshift(event(me, "assignment", item.assignee ? `assigned this to ${item.assignee.name}` : "unassigned this work item"));
      return withLastAction(next, item.assignee ? `Assigned to ${item.assignee.name} (simulated)` : "Work item unassigned (simulated)");
    }
    case "work-item.edit": {
      const item = next.workItems.find((entry) => entry.id === action.workItemId);
      if (!item || !action.title.trim()) return state;
      item.title = action.title.trim();
      item.description = action.description.trim();
      item.timeline.unshift(event(me, "state", "updated the title and description"));
      return withLastAction(next, "Work item updated (simulated)");
    }
    case "work-item.state": {
      const item = next.workItems.find((entry) => entry.id === action.workItemId);
      if (!item) return state;
      item.category = action.category;
      item.state = action.state.trim() || stateName(action.category);
      item.timeline.unshift(event(me, "state", `moved this to ${item.state}`));
      return withLastAction(next, `Moved to ${item.state} (simulated)`);
    }
    case "pipeline.cancel": {
      const pipeline = next.pipelines.find((entry) => entry.id === action.pipelineId);
      if (!pipeline || (pipeline.status !== "running" && pipeline.status !== "queued")) return state;
      pipeline.status = "cancelled";
      pipeline.jobs = pipeline.jobs.map((job) => job.status === "running" || job.status === "queued" ? { ...job, status: "cancelled", duration: "cancelled" } : job);
      pipeline.logs.push("Cancellation requested by Sam Rivera", "Pipeline cancelled (simulated)");
      return withLastAction(next, "Pipeline cancelled (simulated)");
    }
    case "notification.read": {
      const notification = next.notifications.find((entry) => entry.id === action.notificationId);
      if (!notification || !notification.unread) return state;
      notification.unread = false;
      return withLastAction(next, "Notification marked read (simulated)");
    }
    case "notification.read-all":
      next.notifications.forEach((notification) => { notification.unread = false; });
      return withLastAction(next, "All notifications marked read (simulated)");
  }
}

export function unreadNotificationCount(state: DemoState): number {
  return state.notifications.filter((notification) => notification.unread).length;
}

export function pullRequestById(state: DemoState, id: string) {
  return state.pullRequests.find((pullRequest) => pullRequest.id === id);
}

export function workItemById(state: DemoState, id: string) {
  return state.workItems.find((workItem) => workItem.id === id);
}

export function pipelineById(state: DemoState, id: string) {
  return state.pipelines.find((pipeline) => pipeline.id === id);
}
