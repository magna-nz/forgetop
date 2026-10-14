import ReactMarkdown, { type Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { StatusGlyph } from "./ui";

// Forge bodies — PR and work-item descriptions, comments — rendered rather than shown as written,
// following the same conventions as the TUI's `markdown.rs`:
//   - GFM tables, strikethrough and task lists are on.
//   - A line break in the source stays a line break (as GitHub shows a PR body), so plain text —
//     Azure's flattened HTML, a Jira description — reads as it always did.
//   - HTML comments are the prompts of a PR template and are hidden; any other raw HTML is shown
//     as dimmed text, never rendered. react-markdown never emits raw HTML (no
//     `dangerouslySetInnerHTML`), so a stray `<script>` in a comment stays inert.
//   - An image is shown as its alt text linking to the image: forge images usually sit behind the
//     forge's own session, so they'd render broken here, and a link keeps the pane compact.
// Styling lives under `.md` in index.css.

const plugins = [remarkGfm, remarkBreaks, forgeBodies];

/** A node of the markdown syntax tree, as far as the pass below needs to know it. */
interface MdNode {
  type: string;
  value?: string;
  url?: string;
  alt?: string | null;
  children?: MdNode[];
  data?: { hName?: string; hProperties?: Record<string, unknown> };
}

/**
 * A remark pass over the tree. react-markdown already shows raw HTML as escaped text; this pass
 * drops the comments (template prompts) and marks the rest so it can be dimmed, as the TUI shows
 * it. It also turns an image into a link on its alt text — or, inside a link already (a CI
 * badge), into the alt text alone, since an anchor can't nest in an anchor.
 */
function forgeBodies() {
  return (tree: MdNode) => {
    const visit = (node: MdNode, inLink: boolean) => {
      if (!node.children) return;
      node.children = node.children.filter((child) => !(child.type === "html" && isComment(child.value ?? "")));
      node.children = node.children.map((child) => {
        if (child.type === "html") {
          return { type: "text", value: child.value, data: { hName: "span", hProperties: { className: "md-raw" } } };
        }
        if (child.type === "image") {
          const text: MdNode = { type: "text", value: child.alt || "image" };
          return inLink ? text : { type: "link", url: child.url, children: [text] };
        }
        visit(child, inLink || child.type === "link");
        return child;
      });
    };
    visit(tree, false);
  };
}

/** Only a link that leaves the dashboard is worth following; anything else — an in-page
 *  footnote, a relative path, a neutralised `javascript:` — is shown as text. */
function isExternal(href: unknown): href is string {
  return typeof href === "string" && /^(https?:|mailto:)/i.test(href);
}

function isComment(html: string): boolean {
  return html.trimStart().startsWith("<!--");
}

const components: Components = {
  // A link opens beside, not over, the work surface.
  a: ({ node: _node, children, href, ...props }) =>
    isExternal(href) ? (
      <a {...props} href={href} target="_blank" rel="noopener noreferrer">
        {children}
      </a>
    ) : (
      <span className="md-dead-link">{children}</span>
    ),
  // Reached only by a reference-style image (`![alt][ref]`); inline images became links above.
  img: ({ node: _node, src, alt }) => (isExternal(src) ? <a href={src} target="_blank" rel="noopener noreferrer">{alt || "image"}</a> : <span>{alt || "image"}</span>),
  code: ({ node: _node, className, children, ...props }) => (
    <code {...props} className={["mono", className].filter(Boolean).join(" ")}>
      {children}
    </code>
  ),
  // Task-list boxes are state, not controls: a glyph (as a thread's Resolved/Open is), never a
  // form field — a disabled native checkbox renders too washed out to tell ticked from not.
  input: ({ node: _node, type, checked }) =>
    type === "checkbox" ? (
      <span className="md-task" role="checkbox" aria-checked={!!checked} aria-readonly="true" aria-label={checked ? "Done" : "Not done"}>
        <StatusGlyph glyph={checked ? "check-circle" : "circle"} color={checked ? "var(--green)" : "var(--fg3)"} size={13} />
      </span>
    ) : null,
};

/** `source` rendered as markdown. Nothing renders for an empty or whitespace-only body. */
export function Markdown({ source, className }: { source: string | null | undefined; className?: string }) {
  if (!source || !source.trim()) return null;
  return (
    <div className={["md", className].filter(Boolean).join(" ")}>
      <ReactMarkdown remarkPlugins={plugins} components={components}>
        {source}
      </ReactMarkdown>
    </div>
  );
}
