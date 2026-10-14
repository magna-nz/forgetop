import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";

describe("Markdown", () => {
  it("renders the markup rather than showing it", () => {
    const { container } = render(
      <Markdown source={"## Summary\nAdds **idempotency** keys to `POST /charge`.\n\n- [x] replay\n- [ ] TTL\n\n| a | b |\n|---|---|\n| 1 | ~~2~~ |"} />,
    );
    expect(screen.getByRole("heading", { name: "Summary" })).toBeInTheDocument();
    expect(container.querySelector("strong")).toHaveTextContent("idempotency");
    expect(container.querySelector("code")).toHaveClass("mono");
    const boxes = container.querySelectorAll("[role='checkbox']");
    expect(boxes).toHaveLength(2);
    expect(boxes[0]).toHaveAttribute("aria-checked", "true");
    expect(boxes[1]).toHaveAttribute("aria-checked", "false");
    expect(container.querySelector("input")).toBeNull();
    expect(boxes[0].querySelector("svg[data-icon='check-circle']")).not.toBeNull();
    expect(boxes[1].querySelector("svg[data-icon='circle']")).not.toBeNull();
    expect(container.querySelector("table del")).toHaveTextContent("2");
    expect(container.textContent).not.toContain("**");
    expect(container.textContent).not.toContain("- [x]");
  });

  it("keeps a source line break as a line break, so plain text reads as it always did", () => {
    const { container } = render(<Markdown source={"Token expires early.\nSign in\nRefresh"} />);
    expect(container.querySelectorAll("br")).toHaveLength(2);
  });

  it("opens external links beside the dashboard and shows any other link as text", () => {
    const { container } = render(
      <Markdown source={"[docs](https://example.test/x) and [bad](javascript:alert(1)) and https://bare.test/y and [note][^1] [rel](/api/x)\n\n[^1]: a footnote"} />,
    );
    const links = [...container.querySelectorAll("a")];
    expect(links.map((a) => a.getAttribute("href"))).toEqual(["https://example.test/x", "https://bare.test/y"]);
    for (const a of links) {
      expect(a).toHaveAttribute("target", "_blank");
      expect(a).toHaveAttribute("rel", "noopener noreferrer");
    }
    expect(container.textContent).toContain("bad");
    expect(container.textContent).toContain("rel");
    expect(container.querySelector("a[href^='#']")).toBeNull();
    expect(container.querySelector("a[href='']")).toBeNull();
  });

  it("marks a loose task list's items too, so the glyph replaces the bullet", () => {
    const { container } = render(<Markdown source={"- [x] one\n\n- [ ] two"} />);
    const items = container.querySelectorAll("li.task-list-item");
    expect(items).toHaveLength(2);
    expect(items[0].querySelector("p > [role='checkbox']")).not.toBeNull();
  });

  it("hides template comments and shows other raw HTML as dimmed text, never rendered", () => {
    const { container } = render(
      <Markdown source={"<!-- Describe your change -->\nBefore<br>after <b>bold</b>\n\n<script>window.pwned = true</script>\n\n<details><summary>More</summary>\n\nhidden\n\n</details>"} />,
    );
    expect(container.textContent).not.toContain("Describe your change");
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("b")).toBeNull();
    expect(container.querySelector("details")).toBeNull();
    expect((window as unknown as { pwned?: boolean }).pwned).toBeUndefined();
    const raw = [...container.querySelectorAll(".md-raw")].map((n) => n.textContent);
    expect(raw).toContain("<br>");
    expect(raw.some((t) => t?.includes("<script>"))).toBe(true);
    expect(container.textContent).toContain("hidden");
  });

  it("shows an image as a link to it, with its alt text, and a badge as its link's text", () => {
    const { container } = render(<Markdown source={"![screenshot](https://example.test/shot.png)\n\n[![build](https://img.test/b.svg)](https://ci.test)"} />);
    expect(container.querySelector("img")).toBeNull();
    const links = [...container.querySelectorAll("a")];
    expect(links.map((a) => [a.textContent, a.getAttribute("href")])).toEqual([
      ["screenshot", "https://example.test/shot.png"],
      ["build", "https://ci.test"],
    ]);
    expect(container.querySelector("a a")).toBeNull();
  });

  it("renders nothing for an empty body", () => {
    const { container } = render(<Markdown source={"  \n"} />);
    expect(container.innerHTML).toBe("");
  });
});
