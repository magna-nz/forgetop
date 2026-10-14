import { Fragment, type ReactNode } from "react";
import type { HlSpan } from "../types";

/** `text` cut into coloured runs by `spans` — `[len, kind]`, lengths in UTF-16 units, which is
 *  what `slice` counts. Plain runs stay bare text; any other kind gets `.syn-<kind>` from
 *  index.css. Spans that stop short leave the rest of the line plain, and spans that overrun
 *  are clipped, so a stale or mismatched answer can never drop or duplicate characters. */
export function renderSpans(text: string, spans: HlSpan[] | null | undefined): ReactNode {
  if (!spans || spans.length === 0) return text;
  const out: ReactNode[] = [];
  let at = 0;
  spans.forEach(([len, kind], i) => {
    if (!(len > 0) || at >= text.length) return;
    const piece = text.slice(at, at + len);
    at += len;
    out.push(kind === "plain" ? <Fragment key={i}>{piece}</Fragment> : <span key={i} className={`syn-${kind}`}>{piece}</span>);
  });
  if (at < text.length) out.push(<Fragment key="rest">{text.slice(at)}</Fragment>);
  return out;
}
