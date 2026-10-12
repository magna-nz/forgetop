import { ApiError } from "../api";
import { Icon } from "./icons";
import { StateCard } from "./ui";

export function ErrorState({ error }: { error: unknown }) {
  if (error instanceof ApiError && error.status === 401) {
    return (
      <StateCard
        icon={<Icon name="blocked" size={24} />}
        title="Session expired"
        sub="Reopen the dashboard from forgetop (press B in the TUI) to get a fresh link."
      />
    );
  }
  const message = error instanceof Error ? error.message : "Something went wrong.";
  return <StateCard icon={<Icon name="warning" size={24} style={{ color: "var(--yellow)" }} />} title="Couldn't load" sub={message} />;
}
