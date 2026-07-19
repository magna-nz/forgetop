import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { renderWithClient } from "../test/util";
import { Feedback } from "./Feedback";

const status = {
  configured: true,
  diagnostics: {
    size_bytes: 1536,
    oldest_at: "2026-07-19T10:00:00Z",
    newest_at: "2026-07-20T09:00:00Z",
  },
};

function feedbackFetch(options?: {
  configured?: boolean;
  diagnostics?: string;
  postStatus?: number;
}) {
  const posts: unknown[] = [];
  const fn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if ((init?.method ?? "GET") === "POST") {
      posts.push(JSON.parse(String(init?.body)));
      if (options?.postStatus && options.postStatus >= 400) {
        return new Response("Feedback service is temporarily unavailable", {
          status: options.postStatus,
          statusText: "Unavailable",
        });
      }
      return new Response(JSON.stringify({ reference_id: "FB-7H2K" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    if (url.includes("/api/feedback/diagnostics")) {
      return new Response(options?.diagnostics ?? "sanitized diagnostic line", {
        status: 200,
        headers: { "content-type": "text/plain" },
      });
    }
    if (url.includes("/api/feedback/status")) {
      return new Response(JSON.stringify({ ...status, configured: options?.configured ?? true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response("not found", { status: 404 });
  });
  vi.stubGlobal("fetch", fn);
  return { fn, posts };
}

async function ready() {
  expect(await screen.findByRole("heading", { name: "Share feedback" })).toBeInTheDocument();
  await screen.findByText(/1.5 KB/);
}

describe("Feedback", () => {
  it("keeps diagnostic attachment off by default and fetches its preview only on request", async () => {
    const { fn } = feedbackFetch();
    renderWithClient(<Feedback />);
    await ready();

    expect(screen.getByRole("checkbox", { name: /Attach recent diagnostics/ })).not.toBeChecked();
    expect(fn.mock.calls.some(([url]) => String(url).includes("/api/feedback/diagnostics"))).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));

    expect(await screen.findByText("sanitized diagnostic line")).toBeInTheDocument();
    expect(fn.mock.calls.filter(([url]) => String(url).includes("/api/feedback/diagnostics"))).toHaveLength(1);
  });

  it("validates required and bounded fields accessibly", async () => {
    feedbackFetch();
    renderWithClient(<Feedback />);
    await ready();

    await userEvent.click(screen.getByRole("button", { name: "Send feedback" }));
    expect(screen.getByText("Enter a summary.")).toBeInTheDocument();
    expect(screen.getByText("Tell us what happened or what you would like to see.")).toBeInTheDocument();
    expect(screen.getByLabelText("Summary")).toHaveAttribute("aria-invalid", "true");

    fireEvent.change(screen.getByLabelText("Summary"), { target: { value: "x".repeat(121) } });
    await userEvent.type(screen.getByLabelText("Details"), "Useful details");
    await userEvent.click(screen.getByRole("button", { name: "Send feedback" }));
    expect(screen.getByText("Summary must be 120 characters or fewer.")).toBeInTheDocument();
  });

  it("submits the selected category and explicit diagnostic opt-in", async () => {
    const { posts } = feedbackFetch();
    renderWithClient(<Feedback />);
    await ready();

    await userEvent.click(screen.getByRole("radio", { name: /^Idea/ }));
    await userEvent.type(screen.getByLabelText("Summary"), "Add keyboard shortcuts");
    await userEvent.type(screen.getByLabelText("Details"), "A shortcut legend would help.");
    await userEvent.type(screen.getByLabelText(/Contact/), "person@example.com");
    await userEvent.click(screen.getByRole("checkbox", { name: /Attach recent diagnostics/ }));
    await userEvent.click(screen.getByRole("button", { name: "Send feedback" }));

    expect(await screen.findByText("FB-7H2K")).toBeInTheDocument();
    expect(posts).toEqual([
      {
        category: "idea",
        summary: "Add keyboard shortcuts",
        details: "A shortcut legend would help.",
        contact: "person@example.com",
        attach_diagnostics: true,
      },
    ]);
  });

  it("shows an unavailable state without disabling diagnostic review", async () => {
    feedbackFetch({ configured: false });
    renderWithClient(<Feedback />);

    expect(await screen.findByText("Online feedback is not configured")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send feedback" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Preview diagnostics" })).toBeEnabled();
  });

  it("preserves entered feedback after a failed submission and allows retry", async () => {
    const { fn } = feedbackFetch({ postStatus: 503 });
    renderWithClient(<Feedback />);
    await ready();

    await userEvent.type(screen.getByLabelText("Summary"), "Intermittent problem");
    await userEvent.type(screen.getByLabelText("Details"), "This should remain after failure.");
    await userEvent.click(screen.getByRole("button", { name: "Send feedback" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Feedback service is temporarily unavailable");
    expect(screen.getByLabelText("Summary")).toHaveValue("Intermittent problem");
    expect(screen.getByLabelText("Details")).toHaveValue("This should remain after failure.");

    fn.mockImplementationOnce(async () =>
      new Response(JSON.stringify({ reference_id: "FB-RETRY" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.getByText("FB-RETRY")).toBeInTheDocument());
  });
});
