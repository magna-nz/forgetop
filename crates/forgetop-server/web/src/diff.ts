// Minimal unified-diff parser. We only need enough to render a patch with add/remove/context
// colouring and to anchor comments to new-file line numbers (the side providers comment on).

export type DiffLineKind = "hunk" | "context" | "add" | "del" | "meta";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  /** New-file line number (for context + added lines) — the anchor for line comments. */
  newLine?: number;
  /** Old-file line number (context + removed lines) — shown in the gutter. */
  oldLine?: number;
  /** Hunk-header range, present only on `kind: "hunk"` lines — lets a caller work out the
   *  unchanged gaps before/between hunks without re-parsing the header text. */
  oldStart?: number;
  oldCount?: number;
  newStart?: number;
  newCount?: number;
}

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

export function parsePatch(patch: string): DiffLine[] {
  const out: DiffLine[] = [];
  let newLine = 0;
  let oldLine = 0;
  for (const raw of patch.split("\n")) {
    const m = raw.match(HUNK);
    if (m) {
      oldLine = parseInt(m[1], 10);
      // A hunk header omits the count when it's 1 (`@@ -5 +5 @@` means one line, not zero).
      const oldCount = m[2] !== undefined ? parseInt(m[2], 10) : 1;
      newLine = parseInt(m[3], 10);
      const newCount = m[4] !== undefined ? parseInt(m[4], 10) : 1;
      out.push({ kind: "hunk", text: raw, oldStart: oldLine, oldCount, newStart: newLine, newCount });
      continue;
    }
    const c = raw[0];
    if (c === "+") {
      out.push({ kind: "add", text: raw.slice(1), newLine });
      newLine++;
    } else if (c === "-") {
      out.push({ kind: "del", text: raw.slice(1), oldLine });
      oldLine++;
    } else if (c === "\\") {
      // "\ No newline at end of file"
      out.push({ kind: "meta", text: raw });
    } else {
      // context line (leading space, or an empty tail line)
      out.push({ kind: "context", text: raw.startsWith(" ") ? raw.slice(1) : raw, newLine, oldLine });
      newLine++;
      oldLine++;
    }
  }
  // Drop a trailing empty line the split can produce.
  if (out.length && out[out.length - 1].text === "" && out[out.length - 1].kind === "context") out.pop();
  return out;
}
