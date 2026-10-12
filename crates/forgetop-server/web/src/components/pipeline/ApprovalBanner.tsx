// The run pane's approval banner: which gates hold the run and for how long, and — where this
// user can answer them here — Approve / Reject (with an optional inline reason).
import { useState } from "react";
import { fmtSecs } from "../../format";
import type { ApprovalDecision, PipelineApproval, PipelineCapabilities } from "../../types";
import { Icon } from "../icons";
import { Button, StatusGlyph } from "../ui";

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
      className="flex items-center gap-2.5 px-3 py-2.5 flex-wrap"
      style={{
        borderRadius: 6,
        border: "1px solid color-mix(in srgb, var(--yellow) 35%, transparent)",
        background: "color-mix(in srgb, var(--yellow) 6%, transparent)",
      }}
      role="status"
    >
      <StatusGlyph glyph="clock" color="var(--yellow)" />
      <span className="text-[12.5px] min-w-0">
        <span className="font-medium" style={{ color: "var(--fg)" }}>
          {names.join(", ")}
        </span>{" "}
        <span style={{ color: "var(--fg2)" }}>
          {names.length === 1 ? "is" : "are"} waiting for approval
          {waitSecs !== null && <span className="num"> · {fmtSecs(waitSecs)}</span>}
        </span>
      </span>
      <span className="ml-auto flex items-center gap-1.5 flex-wrap">
        {!caps.can_respond_approvals ? (
          runUrl ? (
            <a href={runUrl} target="_blank" rel="noreferrer" className="ft-btn ft-btn-sm">
              Open in {providerLabel} to approve
              <Icon name="external" size={12} />
            </a>
          ) : (
            <span className="text-[12px]" style={{ color: "var(--fg3)" }}>
              Open in {providerLabel} to approve
            </span>
          )
        ) : respondable.length === 0 ? (
          <span className="text-[12px]" style={{ color: "var(--fg3)" }}>
            Waiting on others
          </span>
        ) : (
          respondable.map((a) => {
            const named = respondable.length > 1 ? ` ${a.name}` : "";
            if (rejecting?.id === a.id)
              return (
                <span key={a.id} className="inline-flex items-center gap-1.5">
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
                    className="ft-input"
                    style={{ width: 220, height: 22 }}
                  />
                  <Button sm danger disabled={busy} onClick={() => reject(a, rejecting.comment)}>
                    Confirm reject{named}
                  </Button>
                  <Button sm quiet onClick={() => setRejecting(null)}>
                    Cancel
                  </Button>
                </span>
              );
            return (
              <span key={a.id} className="inline-flex items-center gap-1.5">
                <Button sm disabled={busy} onClick={() => setRejecting({ id: a.id, comment: "" })}>
                  Reject{named}
                </Button>
                <Button sm primary disabled={busy} onClick={() => onRespond(a, "Approve")}>
                  Approve{named}
                </Button>
              </span>
            );
          })
        )}
      </span>
    </div>
  );
}
