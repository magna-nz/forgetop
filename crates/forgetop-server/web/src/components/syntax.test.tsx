import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { renderSpans } from "./syntax";
import type { HlSpan } from "../types";

const line = 'resource "aws_s3_bucket" "logs" {';

describe("renderSpans", () => {
  it("wraps non-plain runs in .syn-<kind> and leaves plain runs bare", () => {
    const spans: HlSpan[] = [
      [8, "keyword"],
      [1, "plain"],
      [15, "str"],
      [1, "plain"],
      [6, "str"],
      [2, "plain"],
    ];
    const { container } = render(<pre>{renderSpans(line, spans)}</pre>);
    expect(container.textContent).toBe(line);
    expect(container.querySelector(".syn-keyword")?.textContent).toBe("resource");
    const strs = [...container.querySelectorAll(".syn-str")].map((e) => e.textContent);
    expect(strs).toEqual(['"aws_s3_bucket"', '"logs"']);
    // Plain text is not wrapped in anything.
    expect(container.querySelectorAll("span")).toHaveLength(3);
  });

  it("returns the bare text when there are no spans", () => {
    expect(renderSpans(line, null)).toBe(line);
    expect(renderSpans(line, undefined)).toBe(line);
    expect(renderSpans(line, [])).toBe(line);
  });

  it("never drops or duplicates characters when spans stop short or overrun", () => {
    const short = render(<pre>{renderSpans(line, [[8, "keyword"]])}</pre>).container;
    expect(short.textContent).toBe(line);
    expect(short.querySelector(".syn-keyword")?.textContent).toBe("resource");
    const over = render(<pre>{renderSpans(line, [[8, "keyword"], [500, "str"], [3, "number"]])}</pre>).container;
    expect(over.textContent).toBe(line);
    expect(over.querySelector(".syn-number")).toBeNull();
  });

  it("counts lengths in UTF-16 units, so an astral character takes two", () => {
    const text = "x = 1 # 🚀 go";
    const spans: HlSpan[] = [
      [4, "plain"],
      [1, "number"],
      [1, "plain"],
      [7, "comment"],
    ];
    const { container } = render(<pre>{renderSpans(text, spans)}</pre>);
    expect(container.textContent).toBe(text);
    expect(container.querySelector(".syn-comment")?.textContent).toBe("# 🚀 go");
  });
});
