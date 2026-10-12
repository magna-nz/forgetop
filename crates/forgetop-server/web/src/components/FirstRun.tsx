import { ConnectionForm } from "./ConnectionForm";
import { Button } from "./ui";

/** Shown when the dashboard opens with no connections configured — the browser equivalent of the
 *  TUI dropping straight into the add-connection wizard on first launch. */
export function FirstRun({ onDone, onSkip }: { onDone: () => void; onSkip: () => void }) {
  return (
    <div className="h-full overflow-auto flex items-start justify-center" style={{ background: "var(--bg)" }}>
      <div className="w-full max-w-lg mx-4 my-[7vh]">
        <h1 className="m-0 mb-1 text-[17px] font-semibold" style={{ color: "var(--fg)", letterSpacing: "-0.012em" }}>
          Welcome to forgetop
        </h1>
        <p className="m-0 text-[13px] mb-5" style={{ color: "var(--fg2)" }}>
          Add your first connection to pull in pull requests, work items, and pipelines. Your token is
          stored in your OS keychain and shared with the terminal app.
        </p>
        <div className="rounded-[10px] p-5" style={{ background: "var(--raised)", border: "1px solid var(--border)" }}>
          <ConnectionForm onSaved={onDone} />
        </div>
        <Button quiet sm onClick={onSkip} className="mt-3">
          Skip for now — I'll add one later in Settings
        </Button>
      </div>
    </div>
  );
}
