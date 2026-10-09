import { describe, expect, it } from "vitest";
import { buildDiffTree, collectFiles, filterTree, findMatches, flattenTree, searchFiles } from "./diffTree";
import type { FileChange } from "./types";

const file = (path: string, extra: Partial<FileChange> = {}): FileChange => ({
  path,
  kind: "Modified",
  additions: 1,
  deletions: 0,
  patch: null,
  ...extra,
});

describe("buildDiffTree", () => {
  it("orders folders (alphabetical) before files (alphabetical) at each level", () => {
    const tree = buildDiffTree([file("zeta.rs"), file("alpha/one.rs"), file("beta.rs")]);
    expect(tree.map((n) => n.name)).toEqual(["alpha", "beta.rs", "zeta.rs"]);
  });

  it("compresses a chain of single-child, file-less directories into one row", () => {
    const tree = buildDiffTree([file("forgetop-core/src/foo.rs"), file("forgetop-core/src/bar.rs")]);
    expect(tree).toHaveLength(1);
    const folder = tree[0];
    expect(folder.type).toBe("folder");
    if (folder.type === "folder") {
      expect(folder.path).toBe("forgetop-core/src");
      expect(folder.name).toBe("forgetop-core/src");
      expect(folder.children.map((c) => c.name)).toEqual(["bar.rs", "foo.rs"]);
      expect(folder.fileCount).toBe(2);
    }
  });

  it("reads an Azure DevOps path's leading slash as no directory", () => {
    const tree = buildDiffTree([file("/src/a.rs"), file("/README.md")]);
    expect(tree.map((n) => n.name)).toEqual(["src", "README.md"]);
  });

  it("does not compress a directory that has sibling files or more than one subdirectory", () => {
    const tree = buildDiffTree([file("a/one.rs"), file("a/sub/two.rs")]);
    expect(tree).toHaveLength(1);
    const folder = tree[0];
    if (folder.type === "folder") {
      expect(folder.name).toBe("a");
      expect(folder.children.map((c) => c.name)).toEqual(["sub", "one.rs"]);
    }
  });

  it("puts root files after root folders", () => {
    const tree = buildDiffTree([file("a.rs"), file("src/b.rs")]);
    expect(tree.map((n) => n.name)).toEqual(["src", "a.rs"]);
  });

  it("aggregates additions/deletions/fileCount up the tree", () => {
    const tree = buildDiffTree([file("src/a.rs", { additions: 3, deletions: 1 }), file("src/b.rs", { additions: 2, deletions: 0 })]);
    const folder = tree[0];
    if (folder.type === "folder") {
      expect(folder.fileCount).toBe(2);
      expect(folder.additions).toBe(5);
      expect(folder.deletions).toBe(1);
    }
  });
});

describe("flattenTree / collectFiles", () => {
  it("descends into a folder only when isOpen says so", () => {
    const tree = buildDiffTree([file("src/a.rs"), file("src/b.rs")]);
    expect(flattenTree(tree, () => false)).toHaveLength(1); // just the folder row
    expect(flattenTree(tree, () => true)).toHaveLength(3); // folder + its 2 files
  });

  it("collects every file in tree order regardless of fold state", () => {
    const tree = buildDiffTree([file("b.rs"), file("a/one.rs")]);
    expect(collectFiles(tree).map((f) => f.path)).toEqual(["a/one.rs", "b.rs"]);
  });
});

describe("searchFiles / findMatches", () => {
  it("is ASCII case-insensitive and counts hits across both the path and the patch", () => {
    const changes = [file("src/Needle.rs", { patch: "+needle again\n+NEEDLE" }), file("src/other.rs", { patch: "+nothing" })];
    const matches = searchFiles(changes, "needle");
    expect(matches.get("src/Needle.rs")).toEqual({ pathHits: 1, patchHits: 2 });
    expect(matches.has("src/other.rs")).toBe(false);
  });

  it("treats an empty/whitespace query as no search, not a universal match", () => {
    expect(searchFiles([file("a.rs")], "  ").size).toBe(0);
  });

  it("findMatches returns non-overlapping ranges", () => {
    expect(findMatches("aaa", "aa")).toEqual([[0, 2]]);
  });
});

describe("filterTree", () => {
  it("keeps only matching files and their ancestor folders, re-aggregating stats over the kept subset", () => {
    const changes = [file("src/a.rs", { additions: 1, patch: "hit" }), file("src/b.rs", { additions: 2 }), file("docs/readme.md", { additions: 5 })];
    const tree = buildDiffTree(changes);
    const matches = searchFiles(changes, "hit");
    const filtered = filterTree(tree, matches);
    expect(filtered.map((n) => n.name)).toEqual(["src"]);
    const srcFolder = filtered[0];
    if (srcFolder.type === "folder") {
      expect(srcFolder.children.map((c) => c.name)).toEqual(["a.rs"]);
      expect(srcFolder.fileCount).toBe(1);
      expect(srcFolder.additions).toBe(1);
    }
  });
});
