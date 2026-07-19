import { useState, type FormEvent } from "react";
import { apiGetText, apiPost, useFeedbackStatus } from "../api";
import type { FeedbackCategory, FeedbackRequest, FeedbackResponse } from "../types";

const CATEGORIES: { value: FeedbackCategory; label: string; hint: string; icon: string }[] = [
  { value: "bug", label: "Bug", hint: "Something is not working", icon: "!" },
  { value: "idea", label: "Idea", hint: "A feature or improvement", icon: "✦" },
  { value: "other", label: "Other", hint: "Anything else to share", icon: "…" },
];

type FormErrors = Partial<Record<"summary" | "details" | "contact", string>>;

const characterCount = (value: string) => Array.from(value).length;

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDiagnosticDate(value: string | null): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function validate(summary: string, details: string, contact: string): FormErrors {
  const errors: FormErrors = {};
  const trimmedSummary = summary.trim();
  const trimmedDetails = details.trim();
  if (!trimmedSummary) errors.summary = "Enter a summary.";
  else if (characterCount(trimmedSummary) > 120) errors.summary = "Summary must be 120 characters or fewer.";
  if (!trimmedDetails) errors.details = "Tell us what happened or what you would like to see.";
  else if (characterCount(trimmedDetails) > 10_000) errors.details = "Details must be 10,000 characters or fewer.";
  if (characterCount(contact.trim()) > 320) errors.contact = "Contact must be 320 characters or fewer.";
  return errors;
}

export function Feedback() {
  const status = useFeedbackStatus();
  const [category, setCategory] = useState<FeedbackCategory>("bug");
  const [summary, setSummary] = useState("");
  const [details, setDetails] = useState("");
  const [contact, setContact] = useState("");
  const [attachDiagnostics, setAttachDiagnostics] = useState(false);
  const [errors, setErrors] = useState<FormErrors>({});
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [referenceId, setReferenceId] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);

  const diagnostics = status.data?.diagnostics;
  const hasDiagnostics = (diagnostics?.size_bytes ?? 0) > 0;
  const canSubmit = status.data?.configured === true;

  const loadPreview = async () => {
    if (previewing || !hasDiagnostics) return;
    setPreviewing(true);
    setPreviewError(null);
    try {
      setPreview(await apiGetText("/api/feedback/diagnostics"));
    } catch (e) {
      setPreviewError(e instanceof Error ? e.message : String(e));
    } finally {
      setPreviewing(false);
    }
  };

  const download = async () => {
    if (downloading || !hasDiagnostics) return;
    setDownloading(true);
    setPreviewError(null);
    try {
      const text = await apiGetText("/api/feedback/diagnostics");
      const url = URL.createObjectURL(new Blob([text], { type: "text/plain;charset=utf-8" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = "forgetop-diagnostics.txt";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch (e) {
      setPreviewError(e instanceof Error ? e.message : String(e));
    } finally {
      setDownloading(false);
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canSubmit || submitting) return;
    const nextErrors = validate(summary, details, contact);
    setErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) return;

    setSubmitting(true);
    setSubmitError(null);
    const request: FeedbackRequest = {
      category,
      summary: summary.trim(),
      details: details.trim(),
      contact: contact.trim() || null,
      attach_diagnostics: attachDiagnostics && hasDiagnostics,
    };
    try {
      const response = await apiPost<FeedbackResponse>("/api/feedback", request);
      setReferenceId(response.reference_id);
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const reset = () => {
    setCategory("bug");
    setSummary("");
    setDetails("");
    setContact("");
    setAttachDiagnostics(false);
    setErrors({});
    setSubmitError(null);
    setReferenceId(null);
  };

  if (referenceId) {
    return (
      <div className="mx-auto max-w-3xl p-5 sm:p-8">
        <section
          className="rounded-xl px-6 py-12 text-center"
          style={{ background: "var(--card)", border: "1px solid var(--border)" }}
        >
          <div
            className="mx-auto mb-4 flex h-11 w-11 items-center justify-center rounded-full text-xl"
            style={{ color: "var(--green)", background: "color-mix(in srgb, var(--green) 14%, transparent)" }}
          >
            ✓
          </div>
          <h2 className="text-lg font-semibold">Thanks for helping improve forgetop</h2>
          <p className="mt-2 text-sm" style={{ color: "var(--dim)" }}>
            Your private feedback reference is
          </p>
          <div
            className="mono mx-auto mt-3 w-fit rounded-md px-3 py-1.5 text-sm"
            style={{ color: "var(--accent)", background: "var(--panel2)", border: "1px solid var(--border)" }}
          >
            {referenceId}
          </div>
          <button
            type="button"
            onClick={reset}
            className="mt-6 rounded-md px-3 py-1.5 text-sm font-medium"
            style={{ color: "var(--fg)", background: "var(--panel2)", border: "1px solid var(--border)" }}
          >
            Send more feedback
          </button>
        </section>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-4xl p-5 sm:p-8">
      <div className="mb-6">
        <h2 className="text-lg font-semibold">Share feedback</h2>
        <p className="mt-1 max-w-2xl text-sm" style={{ color: "var(--dim)" }}>
          Report a problem or suggest what forgetop should do next. Your report is private.
        </p>
      </div>

      {status.error && (
        <div
          role="alert"
          className="mb-5 flex flex-wrap items-center justify-between gap-3 rounded-lg p-3 text-sm"
          style={{ color: "var(--red)", background: "color-mix(in srgb, var(--red) 9%, transparent)", border: "1px solid color-mix(in srgb, var(--red) 35%, var(--border))" }}
        >
          <span>Could not check whether feedback is available. {status.error.message}</span>
          <button
            type="button"
            onClick={() => status.refetch()}
            className="rounded px-2.5 py-1 font-medium"
            style={{ color: "var(--fg)", background: "var(--panel2)", border: "1px solid var(--border)" }}
          >
            Retry
          </button>
        </div>
      )}

      {status.data && !status.data.configured && (
        <div
          className="mb-5 rounded-lg p-3 text-sm"
          style={{ color: "var(--yellow)", background: "color-mix(in srgb, var(--yellow) 9%, transparent)", border: "1px solid color-mix(in srgb, var(--yellow) 35%, var(--border))" }}
        >
          <div className="font-medium">Online feedback is not configured</div>
          <div className="mt-0.5 text-xs opacity-80">
            This build has no private feedback destination. You can still preview or download diagnostics for sharing manually.
          </div>
        </div>
      )}

      <form noValidate onSubmit={submit} className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_17rem]">
        <div
          className="rounded-xl p-4 sm:p-5"
          style={{ background: "var(--card)", border: "1px solid var(--border)" }}
        >
          <fieldset>
            <legend className="text-sm font-semibold">What kind of feedback is this?</legend>
            <div className="mt-3 grid gap-2 sm:grid-cols-3">
              {CATEGORIES.map((item) => {
                const selected = category === item.value;
                return (
                  <label
                    key={item.value}
                    className="flex cursor-pointer gap-2.5 rounded-lg p-3 transition-colors"
                    style={{
                      background: selected ? "color-mix(in srgb, var(--accent) 11%, transparent)" : "var(--panel2)",
                      border: `1px solid ${selected ? "var(--accent)" : "var(--border)"}`,
                    }}
                  >
                    <input
                      type="radio"
                      name="category"
                      value={item.value}
                      checked={selected}
                      onChange={() => setCategory(item.value)}
                      className="mt-0.5"
                    />
                    <span className="min-w-0">
                      <span className="flex items-center gap-1.5 text-sm font-medium">
                        <span aria-hidden="true" style={{ color: selected ? "var(--accent)" : "var(--dim)" }}>{item.icon}</span>
                        {item.label}
                      </span>
                      <span className="mt-0.5 block text-[11px] leading-snug" style={{ color: "var(--dim)" }}>{item.hint}</span>
                    </span>
                  </label>
                );
              })}
            </div>
          </fieldset>

          <div className="mt-5">
            <label htmlFor="feedback-summary" className="text-sm font-medium">Summary</label>
            <input
              id="feedback-summary"
              value={summary}
              required
              onChange={(event) => {
                setSummary(event.target.value);
                if (errors.summary) setErrors((current) => ({ ...current, summary: undefined }));
              }}
              aria-invalid={errors.summary ? "true" : "false"}
              aria-describedby={errors.summary ? "feedback-summary-error" : "feedback-summary-help"}
              className="mt-1.5 w-full rounded-md px-3 py-2 text-sm outline-none"
              style={{ color: "var(--fg)", background: "var(--panel2)", border: `1px solid ${errors.summary ? "var(--red)" : "var(--border)"}` }}
              placeholder="A short description"
            />
            <div className="mt-1 flex justify-between gap-3 text-[11px]" style={{ color: errors.summary ? "var(--red)" : "var(--dim)" }}>
              <span id={errors.summary ? "feedback-summary-error" : "feedback-summary-help"}>{errors.summary ?? "Required"}</span>
              <span className="mono">{characterCount(summary)}/120</span>
            </div>
          </div>

          <div className="mt-4">
            <label htmlFor="feedback-details" className="text-sm font-medium">Details</label>
            <textarea
              id="feedback-details"
              value={details}
              required
              onChange={(event) => {
                setDetails(event.target.value);
                if (errors.details) setErrors((current) => ({ ...current, details: undefined }));
              }}
              rows={8}
              aria-invalid={errors.details ? "true" : "false"}
              aria-describedby={errors.details ? "feedback-details-error" : "feedback-details-help"}
              className="mt-1.5 w-full resize-y rounded-md px-3 py-2 text-sm leading-relaxed outline-none"
              style={{ color: "var(--fg)", background: "var(--panel2)", border: `1px solid ${errors.details ? "var(--red)" : "var(--border)"}` }}
              placeholder="What happened, what did you expect, or what would make forgetop better?"
            />
            <div className="mt-1 flex justify-between gap-3 text-[11px]" style={{ color: errors.details ? "var(--red)" : "var(--dim)" }}>
              <span id={errors.details ? "feedback-details-error" : "feedback-details-help"}>{errors.details ?? "Required"}</span>
              <span className="mono">{characterCount(details).toLocaleString()}/10,000</span>
            </div>
          </div>

          <div className="mt-4">
            <label htmlFor="feedback-contact" className="text-sm font-medium">
              Contact <span className="font-normal" style={{ color: "var(--dim)" }}>(optional)</span>
            </label>
            <input
              id="feedback-contact"
              value={contact}
              onChange={(event) => {
                setContact(event.target.value);
                if (errors.contact) setErrors((current) => ({ ...current, contact: undefined }));
              }}
              aria-invalid={errors.contact ? "true" : "false"}
              aria-describedby="feedback-contact-help"
              className="mt-1.5 w-full rounded-md px-3 py-2 text-sm outline-none"
              style={{ color: "var(--fg)", background: "var(--panel2)", border: `1px solid ${errors.contact ? "var(--red)" : "var(--border)"}` }}
              placeholder="Email or handle"
            />
            <div id="feedback-contact-help" className="mt-1 text-[11px]" style={{ color: errors.contact ? "var(--red)" : "var(--dim)" }}>
              {errors.contact ?? "Only include this if you would like a reply."}
            </div>
          </div>

          {submitError && (
            <div
              role="alert"
              className="mt-5 rounded-lg p-3 text-sm"
              style={{ color: "var(--red)", background: "color-mix(in srgb, var(--red) 9%, transparent)", border: "1px solid color-mix(in srgb, var(--red) 35%, var(--border))" }}
            >
              <div className="font-medium">Feedback was not sent</div>
              <div className="mt-0.5 text-xs opacity-90">{submitError}</div>
            </div>
          )}

          <div className="mt-5 flex flex-wrap items-center justify-between gap-3">
            <span className="text-xs" style={{ color: "var(--dim)" }}>
              Your text stays here if sending fails.
            </span>
            <button
              type="submit"
              disabled={!canSubmit || submitting}
              className="rounded-md px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-50"
              style={{ background: "var(--accent)", color: "#0c1a2b" }}
            >
              {submitting ? "Sending…" : submitError ? "Try again" : "Send feedback"}
            </button>
          </div>
        </div>

        <aside className="flex flex-col gap-4">
          <section
            className="rounded-xl p-4"
            style={{ background: "var(--card)", border: "1px solid var(--border)" }}
          >
            <h3 className="text-sm font-semibold">Recent diagnostics</h3>
            {status.isLoading ? (
              <p className="mt-2 text-xs" style={{ color: "var(--dim)" }}>Checking local diagnostics…</p>
            ) : hasDiagnostics && diagnostics ? (
              <>
                <p className="mt-2 text-xs leading-relaxed" style={{ color: "var(--dim)" }}>
                  {formatBytes(diagnostics.size_bytes)}
                  {formatDiagnosticDate(diagnostics.oldest_at) && <> · from {formatDiagnosticDate(diagnostics.oldest_at)}</>}
                  {formatDiagnosticDate(diagnostics.newest_at) && <> to {formatDiagnosticDate(diagnostics.newest_at)}</>}
                </p>
                <label className="mt-3 flex cursor-pointer items-start gap-2.5 rounded-md p-2.5" style={{ background: "var(--panel2)" }}>
                  <input
                    type="checkbox"
                    checked={attachDiagnostics}
                    onChange={(event) => setAttachDiagnostics(event.target.checked)}
                    className="mt-0.5"
                  />
                  <span>
                    <span className="block text-xs font-medium">Attach recent diagnostics</span>
                    <span className="mt-0.5 block text-[11px]" style={{ color: "var(--dim)" }}>
                      Off by default. Only attached when checked.
                    </span>
                  </span>
                </label>
                <div className="mt-3 flex gap-2">
                  <button
                    type="button"
                    onClick={loadPreview}
                    disabled={previewing}
                    className="flex-1 rounded px-2 py-1.5 text-xs font-medium disabled:opacity-50"
                    style={{ color: "var(--fg)", background: "var(--panel2)", border: "1px solid var(--border)" }}
                  >
                    {previewing ? "Loading…" : previewError ? "Retry preview" : "Preview diagnostics"}
                  </button>
                  <button
                    type="button"
                    onClick={download}
                    disabled={downloading}
                    className="rounded px-2 py-1.5 text-xs font-medium disabled:opacity-50"
                    style={{ color: "var(--fg)", background: "var(--panel2)", border: "1px solid var(--border)" }}
                  >
                    {downloading ? "Saving…" : "Download"}
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="mt-2 text-xs" style={{ color: "var(--dim)" }}>No diagnostics are available to attach.</p>
                <label className="mt-3 flex items-start gap-2.5 opacity-50">
                  <input type="checkbox" disabled />
                  <span className="text-xs">Attach recent diagnostics</span>
                </label>
              </>
            )}
            {previewError && <p role="alert" className="mt-2 text-xs" style={{ color: "var(--red)" }}>{previewError}</p>}
          </section>

          <section
            className="rounded-xl p-4 text-xs leading-relaxed"
            style={{ color: "var(--dim)", background: "var(--card)", border: "1px solid var(--border)" }}
          >
            <h3 className="font-semibold" style={{ color: "var(--fg)" }}>What is sent</h3>
            <p className="mt-2">
              Every report includes the app version, operating system, architecture, selected category, and reference metadata.
            </p>
            <p className="mt-2">
              Diagnostic logs are sent only when you check the attachment box. forgetop does not send automatic telemetry or background reports.
            </p>
          </section>
        </aside>
      </form>

      {preview != null && (
        <section
          aria-label="Diagnostic preview"
          className="mt-5 rounded-xl"
          style={{ background: "var(--card)", border: "1px solid var(--border)" }}
        >
          <div className="flex items-center justify-between gap-3 px-4 py-3" style={{ borderBottom: "1px solid var(--border)" }}>
            <div>
              <h3 className="text-sm font-semibold">Diagnostic preview</h3>
              <p className="text-[11px]" style={{ color: "var(--dim)" }}>This is the sanitized text that can be attached.</p>
            </div>
            <button
              type="button"
              onClick={() => setPreview(null)}
              className="rounded px-2 py-1 text-xs"
              style={{ color: "var(--dim)", background: "var(--panel2)", border: "1px solid var(--border)" }}
            >
              Close preview
            </button>
          </div>
          <pre className="mono max-h-80 overflow-auto whitespace-pre-wrap break-words p-4 text-xs leading-relaxed" style={{ color: "var(--dim)" }}>
            {preview || "The diagnostic snapshot is empty."}
          </pre>
        </section>
      )}
    </div>
  );
}
