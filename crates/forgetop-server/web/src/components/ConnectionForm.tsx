import { useMemo, useState } from "react";
import { apiPost, useProviders } from "../api";
import type { ConnectionRow, FieldKey, ProviderInfo } from "../types";

const SECTION_LABEL: Record<string, string> = {
  pull_requests: "Pull Requests",
  work_items: "Work Items",
  pipelines: "Pipelines",
};

/** Add/edit form for a connection, driven by the provider's field schema from the server.
 *  Reused by the settings page and the first-run wizard. */
export function ConnectionForm({
  initial,
  onSaved,
  onCancel,
}: {
  initial?: ConnectionRow | null;
  onSaved: () => void;
  onCancel?: () => void;
}) {
  const { data: providers } = useProviders();
  const editing = !!initial;
  const [provider, setProvider] = useState<string>(initial?.provider ?? "GitHub");
  const [values, setValues] = useState<Partial<Record<FieldKey, string>>>(() => seedValues(initial));
  const [binds, setBinds] = useState<Set<string>>(() => new Set(initial?.sections ?? []));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const info: ProviderInfo | undefined = useMemo(() => providers?.find((p) => p.provider === provider), [providers, provider]);

  const onProvider = (p: string) => {
    setProvider(p);
    // Reset to the new provider's defaults (display name prefilled).
    const next = providers?.find((x) => x.provider === p);
    const seeded: Partial<Record<FieldKey, string>> = {};
    next?.fields.forEach((f) => {
      if (f.default) seeded[f.key] = f.default;
    });
    setValues(seeded);
    setBinds(new Set(next?.sections ?? []));
  };

  const set = (key: FieldKey, v: string) => setValues((prev) => ({ ...prev, [key]: v }));

  const missing = (info?.fields ?? []).filter((f) => f.required && !f.secret && !(values[f.key] ?? "").trim());
  const needsToken = (info?.fields ?? []).some((f) => f.secret) && !editing && !(values.pat ?? "").trim();

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await apiPost("/api/connections", {
        id: initial?.id,
        provider,
        display_name: values.display_name ?? provider,
        base_url: values.base_url,
        organization: values.organization,
        project: values.project,
        repository: values.repository,
        username: values.username,
        token: (values.pat ?? "").trim() || undefined,
        bind_pull_requests: binds.has("pull_requests"),
        bind_work_items: binds.has("work_items"),
        bind_pipelines: binds.has("pipelines"),
      });
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <label className="flex flex-col gap-1">
        <span className="text-xs font-medium" style={{ color: "var(--dim)" }}>Provider</span>
        <select
          value={provider}
          onChange={(e) => onProvider(e.target.value)}
          disabled={editing}
          className="rounded-md px-3 py-2 text-sm outline-none"
          style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)", opacity: editing ? 0.7 : 1 }}
        >
          {(providers ?? []).map((p) => (
            <option key={p.provider} value={p.provider}>{p.label}</option>
          ))}
        </select>
      </label>

      {(info?.fields ?? []).map((f) => (
        <label key={f.key} className="flex flex-col gap-1">
          <span className="text-xs font-medium" style={{ color: "var(--dim)" }}>
            {f.label}
            {f.required && !f.secret && <span style={{ color: "var(--red)" }}> *</span>}
          </span>
          <input
            type={f.secret ? "password" : "text"}
            value={values[f.key] ?? ""}
            onChange={(e) => set(f.key, e.target.value)}
            placeholder={f.secret && editing ? "Leave blank to keep the current token" : ""}
            autoComplete={f.secret ? "new-password" : "off"}
            className="rounded-md px-3 py-2 text-sm outline-none"
            style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)" }}
          />
          <span className="text-xs" style={{ color: "var(--dim)" }}>{f.help}</span>
        </label>
      ))}

      {(info?.sections.length ?? 0) > 0 && (
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium" style={{ color: "var(--dim)" }}>Show in</span>
          <div className="flex flex-wrap gap-3">
            {(info?.sections ?? []).map((s) => (
              <label key={s} className="flex items-center gap-1.5 text-sm cursor-pointer" style={{ color: "var(--fg)" }}>
                <input
                  type="checkbox"
                  checked={binds.has(s)}
                  onChange={(e) =>
                    setBinds((prev) => {
                      const next = new Set(prev);
                      if (e.target.checked) next.add(s);
                      else next.delete(s);
                      return next;
                    })
                  }
                />
                {SECTION_LABEL[s] ?? s}
              </label>
            ))}
          </div>
        </div>
      )}

      {error && <div className="text-xs" style={{ color: "var(--red)" }}>{error}</div>}

      <div className="flex items-center gap-2 pt-1">
        <button
          onClick={save}
          disabled={busy || missing.length > 0 || needsToken}
          className="rounded-md px-3.5 py-2 text-sm font-medium"
          style={{
            background: "var(--accent)",
            color: "#0c1a2b",
            opacity: busy || missing.length > 0 || needsToken ? 0.5 : 1,
            cursor: busy || missing.length > 0 || needsToken ? "not-allowed" : "pointer",
          }}
        >
          {busy ? "Saving…" : editing ? "Save changes" : "Add connection"}
        </button>
        {onCancel && (
          <button onClick={onCancel} className="rounded-md px-3.5 py-2 text-sm" style={{ color: "var(--dim)", border: "1px solid var(--border)" }}>
            Cancel
          </button>
        )}
        <span className="text-xs ml-auto" style={{ color: "var(--dim)" }}>Token is stored in your OS keychain.</span>
      </div>
    </div>
  );
}

function seedValues(initial?: ConnectionRow | null): Partial<Record<FieldKey, string>> {
  if (!initial) return { display_name: "GitHub" };
  return {
    display_name: initial.display_name,
    base_url: initial.base_url ?? "",
    organization: initial.organization ?? "",
    project: initial.project ?? "",
    repository: initial.repository ?? "",
    username: initial.username ?? "",
    // never prefill the token
  };
}
