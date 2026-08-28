import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const sourceDirectory = dirname(fileURLToPath(import.meta.url));

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((entry) => {
    const path = join(directory, entry);
    return statSync(path).isDirectory() ? sourceFiles(path) : path.endsWith(".tsx") || path.endsWith(".ts") ? [path] : [];
  }).filter((path) => !path.endsWith(".test.ts") && !path.endsWith(".test.tsx"));
}

describe("public demo isolation contract", () => {
  it("does not connect to the app, network, or browser persistence", () => {
    const source = sourceFiles(sourceDirectory).map((path) => readFileSync(path, "utf8")).join("\n");

    expect(source).not.toMatch(/\.\.\/crates\//);
    expect(source).not.toMatch(/\b(fetch|XMLHttpRequest|WebSocket)\s*\(/);
    expect(source).not.toMatch(/\b(localStorage|sessionStorage|indexedDB)\b/);
  });
});
