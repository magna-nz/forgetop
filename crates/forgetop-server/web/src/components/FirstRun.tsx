import { ConnectionForm } from "./ConnectionForm";

/** Shown when the dashboard opens with no connections configured — the browser equivalent of the
 *  TUI dropping straight into the add-connection wizard on first launch. */
export function FirstRun({ onDone, onSkip }: { onDone: () => void; onSkip: () => void }) {
  return (
    <div className="h-full overflow-auto flex items-start justify-center" style={{ background: "var(--bg)" }}>
      <div className="w-full max-w-lg mx-4 my-[7vh]">
        <div className="flex items-center gap-2 mb-1">
          <span className="text-xl" style={{ color: "var(--accent)" }}>◿</span>
          <span className="text-lg font-semibold" style={{ color: "var(--fg)" }}>Welcome to forgetop</span>
        </div>
        <p className="text-sm mb-5" style={{ color: "var(--dim)" }}>
          Add your first connection to pull in pull requests, work items, and pipelines. Your token is
          stored in your OS keychain and shared with the terminal app.
        </p>
        <div className="rounded-xl p-5" style={{ background: "var(--panel)", border: "1px solid var(--border)" }}>
          <ConnectionForm onSaved={onDone} />
        </div>
        <button onClick={onSkip} className="mt-3 text-xs" style={{ color: "var(--dim)" }}>
          Skip for now — I'll add one later in Settings
        </button>
      </div>
    </div>
  );
}
