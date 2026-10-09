// Builds the Files-tab's folder tree from a flat `FileChange[]`, and the pure search logic that
// filters it. Kept free of React so the tree shape and the matching rules are each testable on
// their own, independent of how `PrDetail.tsx` renders them.

import type { FileChange } from "./types";

export interface DiffTreeFile {
  type: "file";
  /** Full path, e.g. "crates/forgetop-core/src/lib.rs" — the file's key. */
  path: string;
  /** Last path segment — the row label. */
  name: string;
  file: FileChange;
}

export interface DiffTreeFolder {
  type: "folder";
  /** Full path from the tree root, through any compressed chain, e.g. "forgetop-core/src" —
   *  the folder's key (used for open/closed state; stable across re-renders). */
  path: string;
  /** Display label — same as `path` for a compressed chain ("forgetop-core/src"). */
  name: string;
  children: DiffTreeNode[];
  /** Total files anywhere under this folder (not just direct children). */
  fileCount: number;
  additions: number;
  deletions: number;
}

export type DiffTreeNode = DiffTreeFile | DiffTreeFolder;

// Ordinal compare — deliberately not `localeCompare`, so ordering can't drift with locale.
const cmp = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

interface RawDir {
  dirs: Map<string, RawDir>;
  files: FileChange[];
}

const emptyDir = (): RawDir => ({ dirs: new Map(), files: [] });

function insert(root: RawDir, file: FileChange) {
  // Azure DevOps paths lead with `/`; an empty segment names no directory.
  const parts = file.path.split("/").filter((p) => p !== "");
  parts.pop(); // the file name itself doesn't route into a subdirectory
  let cur = root;
  for (const part of parts) {
    let next = cur.dirs.get(part);
    if (!next) {
      next = emptyDir();
      cur.dirs.set(part, next);
    }
    cur = next;
  }
  cur.files.push(file);
}

function statsOf(nodes: DiffTreeNode[]): { fileCount: number; additions: number; deletions: number } {
  let fileCount = 0;
  let additions = 0;
  let deletions = 0;
  for (const n of nodes) {
    if (n.type === "file") {
      fileCount += 1;
      additions += n.file.additions;
      deletions += n.file.deletions;
    } else {
      fileCount += n.fileCount;
      additions += n.additions;
      deletions += n.deletions;
    }
  }
  return { fileCount, additions, deletions };
}

function dirToNodes(dir: RawDir, basePath: string): DiffTreeNode[] {
  const folders: DiffTreeFolder[] = [];
  for (const name of [...dir.dirs.keys()].sort(cmp)) {
    folders.push(buildFolder(dir.dirs.get(name)!, basePath ? `${basePath}/${name}` : name, name));
  }
  const files: DiffTreeFile[] = dir.files
    .slice()
    .sort((a, b) => cmp(a.path, b.path))
    .map((file) => ({ type: "file", path: file.path, name: file.path.slice(file.path.lastIndexOf("/") + 1), file }));
  return [...folders, ...files];
}

function buildFolder(dir: RawDir, path: string, name: string): DiffTreeFolder {
  let curDir = dir;
  let curPath = path;
  let curName = name;
  // Compress a chain of directories that each have no sibling files and exactly one
  // subdirectory into a single row ("forgetop-core/src" rather than two empty-looking rows).
  while (curDir.files.length === 0 && curDir.dirs.size === 1) {
    const [childName, childDir] = [...curDir.dirs.entries()][0];
    curPath = `${curPath}/${childName}`;
    curName = `${curName}/${childName}`;
    curDir = childDir;
  }
  const children = dirToNodes(curDir, curPath);
  return { type: "folder", path: curPath, name: curName, children, ...statsOf(children) };
}

/** Builds the Files-tab tree: at each level, folders (alphabetical) before files (alphabetical);
 *  root files sort after root folders. */
export function buildDiffTree(changes: FileChange[]): DiffTreeNode[] {
  const root = emptyDir();
  for (const f of changes) insert(root, f);
  return dirToNodes(root, "");
}

export interface FlatRow {
  depth: number;
  node: DiffTreeNode;
}

/** Flattens the tree into rows for rendering, descending into a folder only when `isOpen` says
 *  so for its path. */
export function flattenTree(nodes: DiffTreeNode[], isOpen: (path: string) => boolean, depth = 0): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const node of nodes) {
    rows.push({ depth, node });
    if (node.type === "folder" && isOpen(node.path)) {
      rows.push(...flattenTree(node.children, isOpen, depth + 1));
    }
  }
  return rows;
}

/** Every file in the tree, in tree order (folders before files, alphabetical, depth-first) —
 *  independent of fold state. Used to pick "the first match" deterministically. */
export function collectFiles(nodes: DiffTreeNode[]): DiffTreeFile[] {
  const out: DiffTreeFile[] = [];
  for (const node of nodes) {
    if (node.type === "file") out.push(node);
    else out.push(...collectFiles(node.children));
  }
  return out;
}

/** Keeps only the files present in `matches` and the folders on their path, re-aggregating each
 *  kept folder's stats over just the kept subset. */
export function filterTree(nodes: DiffTreeNode[], matches: Map<string, FileMatch>): DiffTreeNode[] {
  const out: DiffTreeNode[] = [];
  for (const node of nodes) {
    if (node.type === "file") {
      if (matches.has(node.path)) out.push(node);
    } else {
      const children = filterTree(node.children, matches);
      if (children.length > 0) out.push({ ...node, children, ...statsOf(children) });
    }
  }
  return out;
}

// ---- search ----

export interface FileMatch {
  pathHits: number;
  patchHits: number;
}

function asciiLower(s: string): string {
  return s.replace(/[A-Z]/g, (c) => String.fromCharCode(c.charCodeAt(0) + 32));
}

/** Non-overlapping `[start, end)` ranges where `query` occurs in `text`, ASCII case-insensitive
 *  (locale-independent — a search hit can't change with the user's locale). */
export function findMatches(text: string, query: string): [number, number][] {
  if (!query) return [];
  const hay = asciiLower(text);
  const needle = asciiLower(query);
  const out: [number, number][] = [];
  let idx = 0;
  for (;;) {
    const at = hay.indexOf(needle, idx);
    if (at === -1) break;
    out.push([at, at + needle.length]);
    idx = at + needle.length;
  }
  return out;
}

const countHits = (text: string, query: string) => findMatches(text, query).length;

/** Which files match `query` — in their path or their patch text — and how many times. Only
 *  files with at least one hit are present in the result; an empty/whitespace query matches
 *  nothing (the caller treats that as "not searching", not "match everything"). */
export function searchFiles(changes: FileChange[], query: string): Map<string, FileMatch> {
  const out = new Map<string, FileMatch>();
  const q = query.trim();
  if (!q) return out;
  for (const f of changes) {
    const pathHits = countHits(f.path, q);
    const patchHits = f.patch ? countHits(f.patch, q) : 0;
    if (pathHits > 0 || patchHits > 0) out.set(f.path, { pathHits, patchHits });
  }
  return out;
}
