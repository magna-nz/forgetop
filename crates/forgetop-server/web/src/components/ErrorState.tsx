import { ApiError } from "../api";
import { StateCard } from "./ui";

export function ErrorState({ error }: { error: unknown }) {
  if (error instanceof ApiError && error.status === 401) {
    return <StateCard icon="⚿" title="Session expired" sub="Reopen the dashboard from forgetop (press B in the TUI) to get a fresh link." />;
  }
  const message = error instanceof Error ? error.message : "Something went wrong.";
  return <StateCard icon="⚠" title="Couldn't load" sub={message} />;
}
