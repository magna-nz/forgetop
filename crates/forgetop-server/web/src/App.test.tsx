import { afterEach, describe, expect, it } from "vitest";
import { sectionFromHash, shouldShowFirstRun } from "./App";

describe("feedback route", () => {
  afterEach(() => {
    window.location.hash = "";
  });

  it("recognises the feedback deep link", () => {
    window.location.hash = "#feedback";
    expect(sectionFromHash()).toBe("feedback");
  });

  it("keeps feedback reachable before the first connection is configured", () => {
    expect(shouldShowFirstRun("feedback", 0, false)).toBe(false);
    expect(shouldShowFirstRun("launchpad", 0, false)).toBe(true);
  });
});
