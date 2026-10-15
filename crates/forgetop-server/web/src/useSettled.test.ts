import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useSettled } from "./useSettled";

describe("useSettled", () => {
  afterEach(() => vi.useRealTimers());

  it("answers a value only once it has held for the delay, the first one included", () => {
    vi.useFakeTimers();
    const { result } = renderHook(({ v }) => useSettled(v, 300), { initialProps: { v: "a" } });
    expect(result.current).toBeUndefined();
    act(() => vi.advanceTimersByTime(299));
    expect(result.current).toBeUndefined();
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe("a");
  });

  it("a value that keeps moving never settles; where it stops does", () => {
    vi.useFakeTimers();
    const seen: (string | undefined)[] = [];
    const { result, rerender } = renderHook(({ v }) => {
      const s = useSettled(v, 300);
      seen.push(s);
      return s;
    }, { initialProps: { v: "row-0" } });
    // A held key: a new value every 30ms, twenty times.
    for (let i = 1; i <= 20; i++) {
      act(() => vi.advanceTimersByTime(30));
      rerender({ v: `row-${i}` });
    }
    expect(seen.every((s) => s === undefined)).toBe(true);
    act(() => vi.advanceTimersByTime(300));
    expect(result.current).toBe("row-20");
    expect(new Set(seen.filter(Boolean))).toEqual(new Set(["row-20"]));
  });
});
