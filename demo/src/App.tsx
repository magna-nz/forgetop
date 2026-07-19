import { useEffect, useReducer, useRef, useState } from "react";
import { DashboardShell, DemoNotice, DemoSidebar, DemoTopBar, type DemoSection } from "./components/DashboardShell";
import { Chip, DetailDrawer, EmptyState, List, ListRow, PageHeader, SectionCard, StatCard, StatusBadge } from "./components/primitives";
import { createDemoState, pipelineById, pullRequestById, reduceDemo, unreadNotificationCount, workItemById } from "./demoStore";
import type { DemoPipeline, DemoPullRequest, DemoState, DemoWorkItem, PipelineStatus, PullRequestStatus, WorkItemCategory } from "./demoTypes";

type Selection = { kind: "pr" | "work-item" | "pipeline"; id: string } | null;
type PrView = "all" | "yours" | "merged" | "review";

const prTone: Record<PullRequestStatus, "green" | "yellow" | "purple" | "neutral"> = {
  open: "green", draft: "neutral", merged: "purple", closed: "neutral",
};
const pipelineTone: Record<PipelineStatus, "green" | "yellow" | "red" | "blue" | "neutral"> = {
  passed: "green", failed: "red", running: "blue", queued: "yellow", cancelled: "neutral",
};
const workTone: Record<WorkItemCategory, "green" | "yellow" | "red" | "blue" | "neutral"> = {
  done: "green", blocked: "red", in_progress: "blue", todo: "yellow", backlog: "neutral",
};

function matches(search: string, ...values: string[]) {
  const query = search.trim().toLowerCase();
  return !query || values.join(" ").toLowerCase().includes(query);
}

function titleCase(value: string) {
  return value.replace(/_/g, " ").replace(/\b\w/g, (letter: string) => letter.toUpperCase());
}

function DemoButton({ children, onClick, tone = "default", disabled = false }: { children: React.ReactNode; onClick: () => void; tone?: "default" | "primary" | "danger" | "success"; disabled?: boolean }) {
  return <button type="button" className={`demo-button is-${tone}`} disabled={disabled} onClick={onClick}>{children}</button>;
}

export function App() {
  const [state, dispatch] = useReducer(reduceDemo, undefined, createDemoState);
  const [section, setSection] = useState<DemoSection>("launchpad");
  const [search, setSearch] = useState("");
  const [theme, setTheme] = useState<"slate" | "light">("slate");
  const [selection, setSelection] = useState<Selection>(null);
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [prView, setPrView] = useState<PrView>("all");
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen(true);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const select = (kind: NonNullable<Selection>["kind"], id: string) => setSelection({ kind, id });
  const reset = () => { dispatch({ type: "reset" }); setSelection(null); setSearch(""); setPrView("all"); setNotificationsOpen(false); setCommandOpen(false); };
  const runCommand = (command: DemoSection | "reset" | "search") => {
    if (command === "reset") { reset(); return; }
    if (command === "search") { setCommandOpen(false); requestAnimationFrame(() => searchRef.current?.focus()); return; }
    setSection(command); setSearch(""); setCommandOpen(false);
  };
  const counts = {
    launchpad: state.launchpad.reviewRequested.length + state.launchpad.needsAttention.length,
    "pull-requests": state.pullRequests.filter((item) => item.status === "open").length,
    "work-items": state.workItems.filter((item) => item.category === "in_progress").length,
    pipelines: state.pipelines.filter((item) => item.status === "failed" || item.status === "running").length,
  };

  return (
    <div data-theme={theme}>
      <DashboardShell
        sidebar={<DemoSidebar section={section} onSectionChange={(next) => { setSection(next); setSearch(""); }} counts={counts} onReset={reset} />}
        topbar={<DemoTopBar search={search} onSearchChange={setSearch} searchInputRef={searchRef} notificationCount={unreadNotificationCount(state)} onNotifications={() => setNotificationsOpen((open) => !open)} theme={theme} onThemeToggle={() => setTheme((current) => current === "slate" ? "light" : "slate")} />}
      >
        <DemoNotice />
        <p className="demo-action-feedback" role="status">{state.lastAction ?? "No actions yet"}</p>
        {section === "launchpad" && <Launchpad state={state} search={search} onSelect={select} />}
        {section === "pull-requests" && <PullRequests state={state} search={search} view={prView} onView={setPrView} onSelect={(id) => select("pr", id)} />}
        {section === "work-items" && <WorkItems state={state} search={search} onSelect={(id) => select("work-item", id)} />}
        {section === "pipelines" && <Pipelines state={state} search={search} onSelect={(id) => select("pipeline", id)} />}
        {notificationsOpen && <Notifications state={state} onClose={() => setNotificationsOpen(false)} onRead={(id) => dispatch({ type: "notification.read", notificationId: id })} onReadAll={() => dispatch({ type: "notification.read-all" })} onOpen={(target, id) => { dispatch({ type: "notification.read", notificationId: id }); setNotificationsOpen(false); setSection(target === "pull_request" ? "pull-requests" : target === "work_item" ? "work-items" : "pipelines"); select(target === "pull_request" ? "pr" : target === "work_item" ? "work-item" : "pipeline", state.notifications.find((notice) => notice.id === id)?.targetId ?? ""); }} />}
        <Detail state={state} selection={selection} onClose={() => setSelection(null)} dispatch={dispatch} />
        <CommandPalette open={commandOpen} onClose={() => setCommandOpen(false)} onRun={runCommand} />
      </DashboardShell>
    </div>
  );
}

function Launchpad({ state, search, onSelect }: { state: DemoState; search: string; onSelect: (kind: NonNullable<Selection>["kind"], id: string) => void }) {
  const prs = (ids: string[]) => state.pullRequests.filter((item) => ids.includes(item.id) && matches(search, item.title, item.provider));
  const work = state.workItems.filter((item) => state.launchpad.assignedWork.includes(item.id) && matches(search, item.title, item.identifier));
  const pipes = state.pipelines.filter((item) => state.launchpad.pipelineAlerts.includes(item.id) && matches(search, item.name));
  return <>
    <PageHeader eyebrow="COMMAND CENTER" title="Things that need your attention." description="A simulated cross-provider action inbox." />
    <div className="demo-stat-grid">
      <StatCard label="Needs your review" value={prs(state.launchpad.reviewRequested).length} detail="Pull requests" tone="yellow" />
      <StatCard label="Your work" value={work.length} detail="Assigned work items" tone="blue" />
      <StatCard label="Needs fixing" value={prs(state.launchpad.needsAttention).length + pipes.length} detail="PRs and pipelines" tone="red" />
    </div>
    <div className="demo-launchpad-grid">
      <LaunchpadBucket title="Needs your review" items={prs(state.launchpad.reviewRequested)} onSelect={(id) => onSelect("pr", id)} />
      <LaunchpadBucket title="Your work" items={work} onSelect={(id) => onSelect("work-item", id)} />
      <LaunchpadBucket title="Needs fixing" items={prs(state.launchpad.needsAttention)} onSelect={(id) => onSelect("pr", id)} />
      <LaunchpadBucket title="Pipeline alerts" items={pipes} onSelect={(id) => onSelect("pipeline", id)} />
    </div>
  </>;
}

function LaunchpadBucket({ title, items, onSelect }: { title: string; items: Array<DemoPullRequest | DemoWorkItem | DemoPipeline>; onSelect: (id: string) => void }) {
  return <SectionCard title={title}>{items.length === 0 ? <EmptyState title="All clear" description="No matching simulated work." /> : <List>{items.map((item) => {
    const isPr = "number" in item; const isWork = "identifier" in item;
    return <ListRow key={item.id} title={isPr ? item.title : isWork ? item.title : item.name} subtitle={isPr ? `${item.provider} · #${item.number} · ${item.repository}` : isWork ? `${item.provider} · ${item.identifier}` : `${item.provider} · run #${item.runNumber}`} badge={isPr ? <StatusBadge tone={prTone[item.status]}>{titleCase(item.status)}</StatusBadge> : isWork ? <StatusBadge tone={workTone[item.category]}>{item.state}</StatusBadge> : <StatusBadge tone={pipelineTone[item.status]}>{titleCase(item.status)}</StatusBadge>} onClick={() => onSelect(item.id)} />;
  })}</List>}</SectionCard>;
}

function PullRequests({ state, search, view, onView, onSelect }: { state: DemoState; search: string; view: PrView; onView: (view: PrView) => void; onSelect: (id: string) => void }) {
  const [sort, setSort] = useState<"title" | "repository">("repository");
  const list = state.pullRequests.filter((item) => {
    if (view === "yours" && item.author.id !== state.currentUser.id) return false;
    if (view === "merged" && item.status !== "merged") return false;
    if (view === "review" && !item.reviewers.some((review) => review.reviewer.id === state.currentUser.id && review.vote === "pending")) return false;
    return matches(search, item.title, item.repository, item.provider, item.labels.join(" "));
  }).sort((left, right) => sort === "title" ? left.title.localeCompare(right.title) : left.repository.localeCompare(right.repository) || left.number - right.number);
  return <>
    <PageHeader eyebrow="PULL REQUESTS" title="Review work across your forges" description="All actions are simulated in this browser tab." actions={<><div className="demo-tabs">{(["all", "yours", "review", "merged"] as PrView[]).map((item) => <button key={item} className={view === item ? "is-active" : ""} type="button" onClick={() => onView(item)}>{item === "review" ? "Review requested" : titleCase(item)}</button>)}</div><label className="demo-sort">Sort<select aria-label="Sort pull requests" value={sort} onChange={(event) => setSort(event.target.value as "title" | "repository")}><option value="repository">Repository</option><option value="title">Title</option></select></label></>} />
    <SectionCard title={`${list.length} pull requests`} action={<span className="demo-muted">Filter with search</span>}>{list.length === 0 ? <EmptyState title="No matching pull requests" description="Try another view or search." /> : <List>{list.map((item) => <ListRow key={item.id} leading={<span className="demo-provider">{item.provider.slice(0, 2)}</span>} title={`#${item.number} ${item.title}`} subtitle={`${item.repository} · ${item.author.name} · ${item.updatedAt}`} badge={<StatusBadge tone={prTone[item.status]}>{titleCase(item.status)}</StatusBadge>} meta={<span className={`demo-check is-${item.checks}`}>● {item.checks}</span>} onClick={() => onSelect(item.id)}><Chip>{item.labels[0]}</Chip></ListRow>)}</List>}</SectionCard>
  </>;
}

function WorkItems({ state, search, onSelect }: { state: DemoState; search: string; onSelect: (id: string) => void }) {
  const list = state.workItems.filter((item) => matches(search, item.title, item.identifier, item.provider, item.state, item.labels.join(" ")));
  return <>
    <PageHeader eyebrow="WORK ITEMS" title="Move work forward" description="Sample issues and tasks from four simulated providers." />
    <SectionCard title={`${list.length} work items`} action={<span className="demo-muted">Click an item to edit its simulated state</span>}>{list.length === 0 ? <EmptyState title="No matching work items" /> : <List>{list.map((item) => <ListRow key={item.id} leading={<span className="demo-provider">{item.provider.slice(0, 2)}</span>} title={`${item.identifier} ${item.title}`} subtitle={`${item.project} · ${item.assignee?.name ?? "Unassigned"} · ${item.updatedAt}`} badge={<StatusBadge tone={workTone[item.category]}>{item.state}</StatusBadge>} onClick={() => onSelect(item.id)}>{item.labels.slice(0, 2).map((label) => <Chip key={label}>{label}</Chip>)}</ListRow>)}</List>}</SectionCard>
  </>;
}

function Pipelines({ state, search, onSelect }: { state: DemoState; search: string; onSelect: (id: string) => void }) {
  const list = state.pipelines.filter((item) => matches(search, item.name, item.project, item.provider, item.branch, item.status));
  return <>
    <PageHeader eyebrow="PIPELINES" title="Know what is shipping" description="Open a run for jobs, logs, and simulated cancellation." />
    <SectionCard title={`${list.length} recent runs`}>{list.length === 0 ? <EmptyState title="No matching pipelines" /> : <List>{list.map((item) => <ListRow key={item.id} leading={<span className="demo-provider">{item.provider.slice(0, 2)}</span>} title={`${item.name} · #${item.runNumber}`} subtitle={`${item.branch} · ${item.commit} · ${item.startedAt}`} badge={<StatusBadge tone={pipelineTone[item.status]}>{titleCase(item.status)}</StatusBadge>} onClick={() => onSelect(item.id)} />)}</List>}</SectionCard>
  </>;
}

function Notifications({ state, onClose, onRead, onReadAll, onOpen }: { state: DemoState; onClose: () => void; onRead: (id: string) => void; onReadAll: () => void; onOpen: (target: "pull_request" | "work_item" | "pipeline", id: string) => void }) {
  return <div className="demo-notifications" role="dialog" aria-label="Notifications"><div className="demo-notifications-heading"><strong>Notifications</strong><div><button type="button" onClick={onReadAll}>Mark all read</button><button type="button" onClick={onClose}>×</button></div></div>{state.notifications.map((notice) => <div key={notice.id} className={`demo-notification ${notice.unread ? "is-unread" : ""}`}><button type="button" className="demo-notification-open" onClick={() => onOpen(notice.target, notice.id)}><span className="demo-notification-dot">{notice.unread ? "●" : ""}</span><span><strong>{notice.title}</strong><small>{notice.context}</small><small>{notice.provider} · {notice.updatedAt}</small></span></button>{notice.unread && <button type="button" className="demo-notification-read" onClick={() => onRead(notice.id)}>Mark read</button>}</div>)}</div>;
}

function CommandPalette({ open, onClose, onRun }: { open: boolean; onClose: () => void; onRun: (command: DemoSection | "reset" | "search") => void }) {
  if (!open) return null;
  const commands: Array<{ id: DemoSection | "reset" | "search"; label: string; hint: string }> = [
    { id: "launchpad", label: "Go to Command Center", hint: "Navigation" },
    { id: "pull-requests", label: "Go to Pull Requests", hint: "Navigation" },
    { id: "work-items", label: "Go to Work Items", hint: "Navigation" },
    { id: "pipelines", label: "Go to Pipelines", hint: "Navigation" },
    { id: "search", label: "Focus dashboard search", hint: "Search" },
    { id: "reset", label: "Reset demo", hint: "Demo" },
  ];
  return <div className="demo-command-layer" role="presentation"><button className="demo-drawer-backdrop" type="button" aria-label="Close command palette" onClick={onClose} /><section className="demo-command-palette" role="dialog" aria-modal="true" aria-label="Command palette"><input autoFocus aria-label="Find a command" placeholder="Type a command…" />{commands.map((command) => <button key={command.id} type="button" onClick={() => onRun(command.id)}><span>{command.label}</span><small>{command.hint}</small></button>)}</section></div>;
}

function Detail({ state, selection, onClose, dispatch }: { state: DemoState; selection: Selection; onClose: () => void; dispatch: React.Dispatch<Parameters<typeof reduceDemo>[1]> }) {
  if (!selection) return null;
  if (selection.kind === "pr") { const pr = pullRequestById(state, selection.id); return pr ? <PullRequestDetail pr={pr} currentUser={state.currentUser.name} onClose={onClose} dispatch={dispatch} /> : null; }
  if (selection.kind === "work-item") { const item = workItemById(state, selection.id); return item ? <WorkItemDetail item={item} state={state} onClose={onClose} dispatch={dispatch} /> : null; }
  const pipeline = pipelineById(state, selection.id); return pipeline ? <PipelineDetail pipeline={pipeline} onClose={onClose} dispatch={dispatch} /> : null;
}

function PullRequestDetail({ pr, currentUser, onClose, dispatch }: { pr: DemoPullRequest; currentUser: string; onClose: () => void; dispatch: React.Dispatch<Parameters<typeof reduceDemo>[1]> }) {
  const [comment, setComment] = useState("");
  const [replyTo, setReplyTo] = useState<string | undefined>();
  const [tab, setTab] = useState<"conversation" | "files" | "checks">("conversation");
  const submit = () => { dispatch({ type: "pr.comment", prId: pr.id, body: comment, replyTo }); setComment(""); setReplyTo(undefined); };
  return <DetailDrawer open title={`#${pr.number} ${pr.title}`} subtitle={`${pr.provider} · ${pr.repository} · ${pr.sourceBranch} → ${pr.targetBranch}`} onClose={onClose} footer={<>{pr.status === "open" && <><DemoButton tone="success" onClick={() => dispatch({ type: "pr.review", prId: pr.id, vote: "approved" })}>Approve</DemoButton><DemoButton tone="danger" onClick={() => dispatch({ type: "pr.review", prId: pr.id, vote: "changes_requested" })}>Request changes</DemoButton>{pr.mergeable && pr.checks === "passing" && <DemoButton tone="primary" onClick={() => dispatch({ type: "pr.merge", prId: pr.id })}>Merge</DemoButton>}</>}{pr.status === "merged" && <DemoButton onClick={() => dispatch({ type: "pr.revert", prId: pr.id })}>Revert</DemoButton>}</>}>
    <div className="demo-detail-meta"><StatusBadge tone={prTone[pr.status]}>{titleCase(pr.status)}</StatusBadge><Chip>{pr.checks} checks</Chip><Chip>+{pr.additions} −{pr.deletions}</Chip></div>
    <p className="demo-detail-description">{pr.description}</p>
    <div className="demo-tabs demo-detail-tabs">{(["conversation", "files", "checks"] as const).map((item) => <button key={item} className={tab === item ? "is-active" : ""} type="button" onClick={() => setTab(item)}>{titleCase(item)}</button>)}</div>
    {tab === "conversation" && <><h3>Timeline</h3>{pr.timeline.map((event) => <div className="demo-timeline" key={event.id}><span>{event.actor.initials}</span><p><strong>{event.actor.name}</strong> {event.message}<small>{event.at}</small></p></div>)}<h3>Conversation</h3>{pr.comments.map((item) => <div className="demo-comment" key={item.id}><strong>{item.author.name}</strong><span>{item.createdAt}</span><p>{item.body}</p><button type="button" className="demo-text-button" onClick={() => setReplyTo(item.id)}>Reply</button></div>)}{replyTo && <p className="demo-replying">Replying to a comment <button type="button" className="demo-text-button" onClick={() => setReplyTo(undefined)}>Cancel</button></p>}<textarea aria-label="Add a pull request comment" value={comment} onChange={(event) => setComment(event.target.value)} placeholder={replyTo ? `Reply as ${currentUser}…` : `Comment as ${currentUser}…`} /><DemoButton tone="primary" onClick={submit} disabled={!comment.trim()}>{replyTo ? "Post reply" : "Post comment"}</DemoButton></>}
    {tab === "files" && pr.files.map((file) => <section className="demo-file" key={file.path}><div><strong>{file.path}</strong><span>+{file.additions} −{file.deletions}</span></div><pre>{file.patch}</pre></section>)}
    {tab === "checks" && <List>{pr.reviewers.map((review) => <ListRow key={review.reviewer.id} title={review.reviewer.name} subtitle={review.summary ?? "No review summary"} badge={<StatusBadge tone={review.vote === "approved" ? "green" : review.vote === "changes_requested" ? "red" : "yellow"}>{titleCase(review.vote)}</StatusBadge>} />)}</List>}
  </DetailDrawer>;
}

function WorkItemDetail({ item, state, onClose, dispatch }: { item: DemoWorkItem; state: DemoState; onClose: () => void; dispatch: React.Dispatch<Parameters<typeof reduceDemo>[1]> }) {
  const [title, setTitle] = useState(item.title); const [description, setDescription] = useState(item.description); const [comment, setComment] = useState(""); const [replyTo, setReplyTo] = useState<string | undefined>();
  return <DetailDrawer open title={`${item.identifier} ${item.title}`} subtitle={`${item.provider} · ${item.project}`} onClose={onClose} footer={<><DemoButton tone="primary" onClick={() => dispatch({ type: "work-item.edit", workItemId: item.id, title, description })}>Save changes</DemoButton><DemoButton onClick={onClose}>Close</DemoButton></>}>
    <div className="demo-detail-meta"><StatusBadge tone={workTone[item.category]}>{item.state}</StatusBadge><Chip>{item.type}</Chip>{item.labels.map((label) => <Chip key={label}>{label}</Chip>)}</div>
    <label className="demo-field">Title<input value={title} onChange={(event) => setTitle(event.target.value)} /></label><label className="demo-field">Description<textarea value={description} onChange={(event) => setDescription(event.target.value)} /></label>
    <div className="demo-field-row"><label className="demo-field">Assignee<select value={item.assignee?.id ?? ""} onChange={(event) => dispatch({ type: "work-item.assign", workItemId: item.id, assigneeId: event.target.value || null })}><option value="">Unassigned</option>{state.people.map((person) => <option key={person.id} value={person.id}>{person.name}</option>)}</select></label><label className="demo-field">State<select value={item.category} onChange={(event) => { const category = event.target.value as WorkItemCategory; dispatch({ type: "work-item.state", workItemId: item.id, category, state: titleCase(category) }); }}>{(["backlog", "todo", "in_progress", "done", "blocked"] as WorkItemCategory[]).map((category) => <option key={category} value={category}>{titleCase(category)}</option>)}</select></label></div>
    <h3>Conversation</h3>{item.comments.map((entry) => <div className="demo-comment" key={entry.id}><strong>{entry.author.name}</strong><span>{entry.createdAt}</span><p>{entry.body}</p><button type="button" className="demo-text-button" onClick={() => setReplyTo(entry.id)}>Reply</button></div>)}{replyTo && <p className="demo-replying">Replying to a comment <button type="button" className="demo-text-button" onClick={() => setReplyTo(undefined)}>Cancel</button></p>}<textarea aria-label="Add a work item comment" value={comment} onChange={(event) => setComment(event.target.value)} placeholder={replyTo ? "Add a simulated reply…" : "Add a simulated comment…"} /><DemoButton tone="primary" onClick={() => { dispatch({ type: "work-item.comment", workItemId: item.id, body: comment, replyTo }); setComment(""); setReplyTo(undefined); }} disabled={!comment.trim()}>{replyTo ? "Post reply" : "Post comment"}</DemoButton>
    <h3>Timeline</h3>{item.timeline.map((event) => <div className="demo-timeline" key={event.id}><span>{event.actor.initials}</span><p><strong>{event.actor.name}</strong> {event.message}<small>{event.at}</small></p></div>)}
  </DetailDrawer>;
}

function PipelineDetail({ pipeline, onClose, dispatch }: { pipeline: DemoPipeline; onClose: () => void; dispatch: React.Dispatch<Parameters<typeof reduceDemo>[1]> }) {
  return <DetailDrawer open title={`${pipeline.name} · #${pipeline.runNumber}`} subtitle={`${pipeline.provider} · ${pipeline.branch} · ${pipeline.commit}`} onClose={onClose} footer={pipeline.status === "running" || pipeline.status === "queued" ? <DemoButton tone="danger" onClick={() => dispatch({ type: "pipeline.cancel", pipelineId: pipeline.id })}>Cancel run</DemoButton> : <DemoButton onClick={onClose}>Close</DemoButton>}>
    <div className="demo-detail-meta"><StatusBadge tone={pipelineTone[pipeline.status]}>{titleCase(pipeline.status)}</StatusBadge><Chip>Started {pipeline.startedAt}</Chip></div><h3>Jobs</h3><List>{pipeline.jobs.map((job) => <ListRow key={job.name} title={job.name} subtitle={job.duration} badge={<StatusBadge tone={pipelineTone[job.status]}>{titleCase(job.status)}</StatusBadge>} />)}</List><h3>Logs</h3><pre className="demo-logs">{pipeline.logs.join("\n")}</pre>
  </DetailDrawer>;
}
