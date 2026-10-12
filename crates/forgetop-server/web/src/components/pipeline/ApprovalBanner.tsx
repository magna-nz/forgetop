// The run pane's approval banner: which gates hold the run and for how long, and — where this
// user can answer them here — Approve / Reject (with an optional inline reason).
import { useState } from "react";
import { fmtSecs } from "../../format";
import type { ApprovalDecision, PipelineApproval, PipelineCapabilities } from "../../types";
import { PaneButton } from "./parts";

export function ApprovalBanner({
  approvals,
  caps,
  runUrl,
  providerLabel,
  waitSecs,
  busy,
  onRespond,
}: {
  approvals: PipelineApproval[];
  caps: PipelineCapabilities;
  runUrl: string | null;
  providerLabel: string;
  waitSecs: number | null;
  busy: boolean;
  onRespond: (approval: PipelineApproval, decision: ApprovalDecision, comment?: string) => void;
}) {
  const [rejecting, setRejecting] = useState<{ id: string; comment: string } | null>(null);
  const respondable = caps.can_respond_approvals ? approvals.filter((a) => a.can_respond) : [];
  const names = approvals.map((a) => a.name);
  const reject = (a: PipelineApproval, comment: string) => {
    setRejecting(null);
    onRespond(a, "Reject", comment.trim() || undefined);
  };

  return (
    <div
      className="flex items-center gap-2.5 px-5 py-2 text-[13px] flex-wrap"
      style={{ background: "color-mix(in srgb, var(--yellow) 8%, transparent)", borderBottom: "1px solid color-mix(in srgb, var(--yellow) 30%, transparent)" }}
      role="status"
    >
      <span className="font-bold" style={{ color: "var(--yellow)" }}>
        ⏸ {names.join(", ")} {names.length === 1 ? "needs" : "need"} approval
      </span>
      {waitSecs !== null && <span style={{ color: "var(--yellow)" }}>· waiting {fmtSecs(waitSecs)}</span>}
      <span className="flex-1" />
      {!caps.can_respond_approvals ? (
        runUrl ? (
          <a href={runUrl} target="_blank" rel="noreferrer" className="text-xs underline" style={{ color: "var(--dim)" }}>
            Open in {providerLabel} to approve ↗
          </a>
        ) : (
          <span className="text-xs" style={{ color: "var(--dim)" }}>
            Open in {providerLabel} to approve
          </span>
        )
      ) : respondable.length === 0 ? (
        <span className="text-xs" style={{ color: "var(--dim)" }}>
          Waiting on others
        </span>
      ) : (
        respondable.map((a) => {
          const named = respondable.length > 1 ? ` ${a.name}` : "";
          if (rejecting?.id === a.id)
            return (
              <span key={a.id} className="inline-flex items-center gap-2">
                <input
                  autoFocus
                  value={rejecting.comment}
                  onChange={(e) => setRejecting({ id: a.id, comment: e.target.value })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") reject(a, rejecting.comment);
                    if (e.key === "Escape") {
                      // Closes the reason box, not the pane.
                      e.stopPropagation();
                      setRejecting(null);
                    }
                  }}
                  placeholder="Reason (optional)"
                  aria-label={`Reason for rejecting ${a.name}`}
                  className="rounded-md px-2 py-1 text-xs outline-none"
                  style={{ background: "var(--bg)", border: "1px solid var(--border)", color: "var(--fg)", width: 220 }}
                />
                <PaneButton label={`✗ Confirm reject${named}`} color="var(--red)" disabled={busy} onClick={() => reject(a, rejecting.comment)} />
                <PaneButton label="Cancel" neutral onClick={() => setRejecting(null)} />
              </span>
            );
          return (
            <span key={a.id} className="inline-flex items-center gap-2">
              <PaneButton label={`✓ Approve${named}`} color="var(--green)" filled disabled={busy} onClick={() => onRespond(a, "Approve")} />
              <PaneButton label={`✗ Reject${named}`} color="var(--red)" disabled={busy} onClick={() => setRejecting({ id: a.id, comment: "" })} />
            </span>
          );
        })
      )}
    </div>
  );
}
