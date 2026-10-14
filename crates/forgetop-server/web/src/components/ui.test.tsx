import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  Button,
  ContextMenu,
  copyText,
  IconButton,
  type MenuEntry,
  notify,
  PaneHeader,
  PaneSlotContext,
  Row,
  RowMenuButton,
  Segmented,
  Select,
  SlideOver,
  StatusBadge,
  StatusGlyph,
  Tabs,
  Toasts,
  useContextMenu,
} from "./ui";

describe("Button", () => {
  it("is a real button with one class per variant", () => {
    render(
      <>
        <Button primary>Merge</Button>
        <Button quiet>More</Button>
        <Button danger>Cancel run</Button>
        <Button sm icon="check">
          Approve
        </Button>
      </>,
    );
    expect(screen.getByRole("button", { name: "Merge" })).toHaveClass("ft-btn", "ft-btn-primary");
    expect(screen.getByRole("button", { name: "More" })).toHaveClass("ft-btn-quiet");
    expect(screen.getByRole("button", { name: "Cancel run" })).toHaveClass("ft-btn-danger");
    const approve = screen.getByRole("button", { name: "Approve" });
    expect(approve).toHaveClass("ft-btn-sm");
    expect(approve.querySelector("svg[data-icon='check']")).not.toBeNull();
    expect(approve).toHaveAttribute("type", "button");
  });

  it("icon-only buttons carry their name for assistive tech", () => {
    render(<IconButton icon="refresh" label="Refresh" />);
    const b = screen.getByRole("button", { name: "Refresh" });
    expect(b).toHaveAttribute("title", "Refresh");
    expect(b.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
  });
});

describe("Select, Segmented, Tabs", () => {
  it("Select stays a native select behind the quiet styling", async () => {
    const onChange = vi.fn();
    render(
      <Select label="Show" onChange={(e) => onChange(e.target.value)} defaultValue="open">
        <option value="open">Open</option>
        <option value="all">All</option>
      </Select>,
    );
    const select = screen.getByLabelText("Show");
    expect(select.tagName).toBe("SELECT");
    expect(select).toHaveClass("ft-select");
    await userEvent.selectOptions(select, "all");
    expect(onChange).toHaveBeenCalledWith("all");
  });

  it("Segmented marks the pressed option and reports a change", async () => {
    const onChange = vi.fn();
    render(<Segmented label="Group" value="pipeline" onChange={onChange} options={[{ value: "pipeline", label: "By pipeline" }, { value: "flat", label: "Flat" }]} />);
    const group = screen.getByRole("group", { name: "Group" });
    expect(within(group).getByRole("button", { name: "By pipeline" })).toHaveAttribute("aria-pressed", "true");
    expect(within(group).getByRole("button", { name: "Flat" })).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(within(group).getByRole("button", { name: "Flat" }));
    expect(onChange).toHaveBeenCalledWith("flat");
  });

  it("Tabs expose a tablist with the selected tab and its count", async () => {
    const onChange = vi.fn();
    render(
      <Tabs
        label="Views"
        value="all"
        onChange={onChange}
        tabs={[
          { id: "all", label: "All", count: 9 },
          { id: "mine", label: "Mine", count: 5 },
        ]}
      />,
    );
    const tabs = screen.getAllByRole("tab");
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    expect(tabs[0]).toHaveTextContent("All9");
    await userEvent.click(tabs[1]);
    expect(onChange).toHaveBeenCalledWith("mine");
  });
});

describe("Row and status", () => {
  it("a row with a click handler is a button; with a link it opens the provider in a new tab", () => {
    const onClick = vi.fn();
    render(
      <>
        <Row onClick={onClick} selected>
          Open me
        </Row>
        <Row href="https://example.test/pr/1">Link me</Row>
      </>,
    );
    const button = screen.getByRole("button", { name: "Open me" });
    expect(button).toHaveClass("ft-row");
    expect(button).toHaveAttribute("data-selected", "true");
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalled();
    const link = screen.getByRole("link", { name: "Link me" });
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noreferrer");
  });

  it("a status glyph is decorative unless titled, and the badge pairs it with the word", () => {
    const { container } = render(
      <>
        <StatusGlyph glyph="x-circle" color="red" />
        <StatusGlyph glyph="running" title="running" spin />
        <StatusBadge glyph="check-circle" color="green" label="passing" />
      </>,
    );
    const svgs = container.querySelectorAll("svg");
    expect(svgs[0]).toHaveAttribute("aria-hidden", "true");
    expect(svgs[0]).toHaveAttribute("data-icon", "x-circle");
    expect(screen.getByRole("img", { name: "running" })).toHaveClass("spin");
    expect(screen.getByText("passing").querySelector("svg[data-icon='check-circle']")).not.toBeNull();
  });
});

describe("SlideOver", () => {
  it("renders beside the content through the pane slot, with header, body and footer", () => {
    const slot = document.createElement("div");
    slot.setAttribute("data-testid", "slot");
    document.body.appendChild(slot);
    const onClose = vi.fn();
    render(
      <PaneSlotContext.Provider value={slot}>
        <div data-testid="content">list</div>
        <SlideOver onClose={onClose} width="xl" label="Pull request" header={<PaneHeader glyph="pr-open" color="green" title="Refactor the retry queue" ident="#1501" />} footer={<Button primary>Merge</Button>}>
          body
        </SlideOver>
      </PaneSlotContext.Provider>,
    );
    const pane = screen.getByRole("region", { name: "Pull request" });
    expect(slot).toContainElement(pane);
    expect(pane).toHaveClass("ft-pane", "ft-pane-xl");
    expect(within(pane).getByRole("heading", { name: "Refactor the retry queue" })).toBeInTheDocument();
    expect(within(pane).getByText("#1501")).toHaveClass("mono");
    expect(within(pane).getByRole("button", { name: "Merge" })).toBeInTheDocument();
    // No backdrop: the page content is still there, undimmed.
    expect(screen.getByTestId("content")).toBeVisible();
    slot.remove();
  });

  it("closes on Escape and on the close button; legacy widths map to the new sizes", async () => {
    const onClose = vi.fn();
    render(
      <SlideOver onClose={onClose} width="5xl" header={<span>h</span>}>
        body
      </SlideOver>,
    );
    expect(screen.getByRole("region", { name: "Details" })).toHaveClass("ft-pane-xl");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe("SlideOver resize grip", () => {
  // jsdom has no PointerEvent; a MouseEvent carries the clientX the grip reads.
  if (!("PointerEvent" in window)) (window as unknown as { PointerEvent: typeof MouseEvent }).PointerEvent = MouseEvent;

  it("drags the pane wider, remembers it per size, and resets on double-click", async () => {
    Object.defineProperty(window, "innerWidth", { value: 1400, configurable: true });
    render(
      <SlideOver onClose={() => {}} width="md" header={<span>h</span>}>
        body
      </SlideOver>,
    );
    const pane = screen.getByRole("region", { name: "Details" });
    const grip = screen.getByRole("separator", { name: "Resize pane" });
    expect(pane).not.toHaveStyle({ width: "700px" });
    // jsdom has no layout: the pane's right edge reads as 0, so the viewport's 1400 stands in and
    // a drag computes the width from the pointer — pointer at 700 means a 700px pane.
    fireEvent.pointerDown(grip, { button: 0, pointerId: 1, clientX: 1400 });
    fireEvent.pointerMove(grip, { clientX: 700 });
    fireEvent.pointerUp(grip);
    expect(pane).toHaveStyle({ width: "700px" });
    expect(localStorage.getItem("forgetop:pane:md")).toBe("700");

    // Keyboard: ← expands, → shrinks, by 24px — on the grip, and from anywhere on the page
    // that is not a text field. The footer says so.
    grip.focus();
    fireEvent.keyDown(grip, { key: "ArrowLeft" });
    expect(pane).toHaveStyle({ width: "724px" });
    fireEvent.keyDown(grip, { key: "ArrowRight" });
    fireEvent.keyDown(grip, { key: "ArrowRight" });
    expect(pane).toHaveStyle({ width: "676px" });
    fireEvent.keyDown(document.body, { key: "ArrowLeft" });
    expect(pane).toHaveStyle({ width: "700px" });
    const input = document.createElement("input");
    document.body.appendChild(input);
    fireEvent.keyDown(input, { key: "ArrowLeft" });
    expect(pane).toHaveStyle({ width: "700px" });
    input.remove();
    expect(pane.textContent).toContain("expand");
    expect(pane.textContent).toContain("shrink");

    // The content column keeps at least 320px: a drag past that is clamped.
    fireEvent.pointerDown(grip, { button: 0, pointerId: 1, clientX: 1400 });
    fireEvent.pointerMove(grip, { clientX: 10 });
    fireEvent.pointerUp(grip);
    expect(pane).toHaveStyle({ width: "1080px" });

    fireEvent.doubleClick(grip);
    expect(pane.style.width).toBe("");
    expect(localStorage.getItem("forgetop:pane:md")).toBeNull();
  });

  it("opens at the remembered width for its size only", () => {
    localStorage.setItem("forgetop:pane:xl", "800");
    const { unmount } = render(
      <SlideOver onClose={() => {}} width="xl" header={<span>h</span>}>
        body
      </SlideOver>,
    );
    expect(screen.getByRole("region", { name: "Details" })).toHaveStyle({ width: "800px" });
    unmount();
    render(
      <SlideOver onClose={() => {}} width="md" header={<span>h</span>}>
        body
      </SlideOver>,
    );
    expect(screen.getByRole("region", { name: "Details" }).style.width).toBe("");
  });
});

/** A row wired the way the lists wire it: right-click, Shift+F10 and the ⋯ open one menu. */
function MenuRow({ items, onRowClick = () => {} }: { items: MenuEntry[] | (() => MenuEntry[]); onRowClick?: () => void }) {
  const menu = useContextMenu();
  return (
    <>
      <Row onClick={onRowClick} onContextMenu={menu.onContextMenu} onKeyDown={menu.onKeyDown} menuOpen={menu.open} label="Refactor the retry queue">
        Refactor the retry queue
        <RowMenuButton onOpen={menu.openAt} />
      </Row>
      <ContextMenu anchor={menu.anchor} onClose={menu.close} items={items} label="Pull request actions" />
    </>
  );
}

const rowButton = () => screen.getByRole("button", { name: "Refactor the retry queue" });

describe("ContextMenu", () => {
  it("right-click prevents the native menu, opens the labelled menu and marks the row", () => {
    const onRowClick = vi.fn();
    const items = vi.fn((): MenuEntry[] => [{ label: "Open", hint: "↵" }, { label: "Copy link", hint: "y" }]);
    render(<MenuRow items={items} onRowClick={onRowClick} />);
    expect(items).not.toHaveBeenCalled();
    expect(rowButton()).not.toHaveAttribute("data-menu-open");
    const notPrevented = fireEvent.contextMenu(rowButton(), { clientX: 40, clientY: 60 });
    expect(notPrevented).toBe(false);
    const menu = screen.getByRole("menu", { name: "Pull request actions" });
    expect(document.body).toContainElement(menu);
    expect(within(menu).getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Open↵", "Copy linky"]);
    // The accessible name is the label alone; the hint is announced as a shortcut.
    expect(within(menu).getByRole("menuitem", { name: "Copy link" })).toHaveAttribute("aria-keyshortcuts", "y");
    expect(within(menu).getByRole("menuitem", { name: "Open" })).toHaveFocus();
    expect(rowButton()).toHaveAttribute("data-menu-open", "true");
    expect(items).toHaveBeenCalledTimes(1);
    expect(onRowClick).not.toHaveBeenCalled();
  });

  it("arrow keys wrap, skip separators but not disabled items, and one item is the tab stop", async () => {
    render(<MenuRow items={[{ label: "Open" }, { type: "separator" }, { label: "Approve" }, { label: "Merge", disabled: "Checks failed" }]} />);
    fireEvent.contextMenu(rowButton());
    const item = (name: string) => screen.getByRole("menuitem", { name });
    expect(screen.getAllByRole("separator")).toHaveLength(1);
    expect(item("Open")).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(item("Approve")).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(item("Merge")).toHaveFocus();
    expect(screen.getAllByRole("menuitem").filter((m) => m.tabIndex === 0)).toEqual([item("Merge")]);
    await userEvent.keyboard("{ArrowDown}");
    expect(item("Open")).toHaveFocus();
    await userEvent.keyboard("{ArrowUp}");
    expect(item("Merge")).toHaveFocus();
  });

  it("typing a key jumps to the item with that hint, else the one whose label starts with it", async () => {
    render(<MenuRow items={[{ label: "Open" }, { label: "Copy link", hint: "y" }, { label: "Merge" }, { label: "Close", hint: "X" }]} />);
    fireEvent.contextMenu(rowButton());
    await userEvent.keyboard("y");
    expect(screen.getByRole("menuitem", { name: "Copy link" })).toHaveFocus();
    await userEvent.keyboard("m");
    expect(screen.getByRole("menuitem", { name: "Merge" })).toHaveFocus();
    await userEvent.keyboard("X");
    expect(screen.getByRole("menuitem", { name: "Close" })).toHaveFocus();
  });

  it("Enter runs the item and closes the menu", async () => {
    const onSelect = vi.fn();
    render(<MenuRow items={[{ label: "Open" }, { label: "Copy link", onSelect }]} />);
    fireEvent.contextMenu(rowButton());
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
    expect(rowButton()).not.toHaveAttribute("data-menu-open");
  });

  it("Esc closes and returns focus to the row; Shift+F10 opens it from the row", async () => {
    const onEscapeOutside = vi.fn();
    window.addEventListener("keydown", onEscapeOutside);
    render(<MenuRow items={[{ label: "Open" }]} />);
    rowButton().focus();
    await userEvent.keyboard("{Shift>}{F10}{/Shift}");
    expect(screen.getByRole("menuitem", { name: "Open" })).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(rowButton()).toHaveFocus();
    // The menu's Esc is its own: an open pane's Esc listener on the window never hears it.
    expect(onEscapeOutside.mock.calls.filter(([e]) => (e as KeyboardEvent).key === "Escape")).toHaveLength(0);
    window.removeEventListener("keydown", onEscapeOutside);
  });

  it("a click outside closes it", async () => {
    render(<MenuRow items={[{ label: "Open" }]} />);
    fireEvent.contextMenu(rowButton());
    expect(screen.getByRole("menu")).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("a disabled item keeps its reason as the tooltip, stays focusable and does not run", async () => {
    const onSelect = vi.fn();
    render(<MenuRow items={[{ label: "Merge", disabled: "Checks are still running", onSelect }]} />);
    fireEvent.contextMenu(rowButton());
    const merge = screen.getByRole("menuitem", { name: "Merge" });
    expect(merge).toHaveAttribute("aria-disabled", "true");
    expect(merge).toHaveAttribute("title", "Checks are still running");
    expect(merge).not.toHaveAttribute("disabled");
    expect(merge).toHaveFocus();
    await userEvent.click(merge);
    await userEvent.keyboard("{Enter}");
    expect(onSelect).not.toHaveBeenCalled();
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("a checked entry is a menuitemcheckbox", () => {
    render(<MenuRow items={[{ label: 'Hide "Done" items', checked: true }, { label: 'Hide "Todo" items', checked: false }]} />);
    fireEvent.contextMenu(rowButton());
    expect(screen.getByRole("menuitemcheckbox", { name: 'Hide "Done" items' })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("menuitemcheckbox", { name: 'Hide "Todo" items' })).toHaveAttribute("aria-checked", "false");
  });

  it("a confirm item needs a second click, shows the confirm label in between and stays open", async () => {
    const onSelect = vi.fn();
    render(<MenuRow items={[{ label: "Open" }, { label: "Close pull request", danger: true, confirm: "Click again to close", onSelect }]} />);
    fireEvent.contextMenu(rowButton());
    await userEvent.click(screen.getByRole("menuitem", { name: "Close pull request" }));
    expect(onSelect).not.toHaveBeenCalled();
    const armed = screen.getByRole("menuitem", { name: "Click again to close" });
    expect(armed).toHaveAttribute("data-danger");
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await userEvent.click(armed);
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("an armed confirm reverts after 4 seconds", () => {
    vi.useFakeTimers();
    try {
      const onSelect = vi.fn();
      render(<MenuRow items={[{ label: "Cancel run", danger: true, confirm: "Click again to cancel", onSelect }]} />);
      fireEvent.contextMenu(rowButton());
      fireEvent.click(screen.getByRole("menuitem", { name: "Cancel run" }));
      expect(screen.getByRole("menuitem", { name: "Click again to cancel" })).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(4000);
      });
      fireEvent.click(screen.getByRole("menuitem", { name: "Cancel run" }));
      expect(onSelect).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("a submenu loader runs when it opens, shows Loading… then its items; ← returns to the parent", async () => {
    let resolve: (v: MenuEntry[]) => void = () => {};
    const loader = vi.fn(() => new Promise<MenuEntry[]>((r) => (resolve = r)));
    const onSelect = vi.fn();
    render(<MenuRow items={[{ label: "Open" }, { label: "Request reviewer", icon: "user-plus", submenu: loader }]} />);
    fireEvent.contextMenu(rowButton());
    expect(loader).not.toHaveBeenCalled();
    const parent = screen.getByRole("menuitem", { name: "Request reviewer" });
    expect(parent).toHaveAttribute("aria-haspopup", "menu");
    expect(parent).toHaveAttribute("aria-expanded", "false");
    await userEvent.keyboard("{ArrowDown}{ArrowRight}");
    expect(loader).toHaveBeenCalledTimes(1);
    expect(parent).toHaveAttribute("aria-expanded", "true");
    const sub = screen.getByRole("menu", { name: "Request reviewer" });
    expect(within(sub).getByRole("menuitem", { name: "Loading…" })).toHaveFocus();
    await act(async () => resolve([{ label: "Ada Lovelace", value: "ada", onSelect }]));
    const ada = await screen.findByRole("menuitem", { name: "Ada Lovelace" });
    expect(ada).toHaveAccessibleDescription("ada");
    expect(ada).toHaveFocus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(screen.queryByRole("menu", { name: "Request reviewer" })).toBeNull();
    expect(parent).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    // Reopened from the cache: the loader is not called again within one open menu.
    await userEvent.click(await screen.findByRole("menuitem", { name: "Ada Lovelace" }));
    expect(loader).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("hovering a submenu parent opens it after a beat, without taking focus from the parent", () => {
    vi.useFakeTimers();
    try {
      render(<MenuRow items={[{ label: "Open" }, { label: "Assign", submenu: [{ label: "Unassigned" }] }]} />);
      fireEvent.contextMenu(rowButton());
      const parent = screen.getByRole("menuitem", { name: "Assign" });
      fireEvent.mouseEnter(parent);
      expect(parent).toHaveFocus();
      expect(screen.queryByRole("menu", { name: "Assign" })).toBeNull();
      act(() => {
        vi.advanceTimersByTime(120);
      });
      expect(screen.getByRole("menu", { name: "Assign" })).toBeInTheDocument();
      expect(parent).toHaveFocus();
      // Moving to another item closes it after the same beat.
      fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "Open" }));
      act(() => {
        vi.advanceTimersByTime(120);
      });
      expect(screen.queryByRole("menu", { name: "Assign" })).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("flips up and left at the viewport edge and stays inside it", () => {
    Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 768, configurable: true });
    const real = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("ft-menu") ? new DOMRect(0, 0, 260, 200) : real.call(this);
    });
    render(<MenuRow items={[{ label: "Open" }]} />);
    fireEvent.contextMenu(rowButton(), { clientX: 1000, clientY: 700 });
    expect(screen.getByRole("menu")).toHaveStyle({ left: "740px", top: "500px" });
    expect(screen.getByRole("menu").style.visibility).toBe("");
    fireEvent.mouseDown(document.body);
    fireEvent.contextMenu(rowButton(), { clientX: 30, clientY: 40 });
    expect(screen.getByRole("menu")).toHaveStyle({ left: "30px", top: "40px" });
  });

  it("a submenu whose loader rejects says it couldn't load", async () => {
    render(<MenuRow items={[{ label: "Move to", submenu: () => Promise.reject(new Error("502 from the forge")) }]} />);
    fireEvent.contextMenu(rowButton());
    await userEvent.keyboard("{ArrowRight}");
    const failed = await screen.findByRole("menuitem", { name: "Couldn't load" });
    expect(failed).toHaveAttribute("title", "502 from the forge");
  });
});

describe("RowMenuButton", () => {
  it("click and Enter call onOpen with the element and never reach the row", async () => {
    const onOpen = vi.fn();
    const onRowClick = vi.fn();
    render(
      <Row onClick={onRowClick}>
        title
        <RowMenuButton onOpen={onOpen} />
      </Row>,
    );
    const dots = screen.getByTitle("More actions");
    expect(dots.tagName).toBe("SPAN");
    expect(dots).toHaveAttribute("tabindex", "-1");
    expect(dots).toHaveAttribute("aria-hidden", "true");
    expect(dots).toHaveClass("ft-row-menu");
    await userEvent.click(dots);
    expect(onOpen).toHaveBeenCalledWith(dots);
    dots.focus();
    await userEvent.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledTimes(2);
    expect(onRowClick).not.toHaveBeenCalled();
  });

  it("opens the row's menu anchored to itself, and focus returns to it", async () => {
    const onRowClick = vi.fn();
    render(<MenuRow items={[{ label: "Open" }]} onRowClick={onRowClick} />);
    const dots = screen.getByTitle("More actions");
    await userEvent.click(dots);
    expect(screen.getByRole("menuitem", { name: "Open" })).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(dots).toHaveFocus();
    expect(onRowClick).not.toHaveBeenCalled();
  });
});

describe("Row menu state", () => {
  it("menuOpen sets data-menu-open", () => {
    render(
      <>
        <Row onClick={() => {}} menuOpen>
          Open one
        </Row>
        <Row href="https://example.test/pr/1" menuOpen>
          Linked one
        </Row>
        <Row onClick={() => {}}>Closed one</Row>
      </>,
    );
    expect(screen.getByRole("button", { name: "Open one" })).toHaveAttribute("data-menu-open", "true");
    expect(screen.getByRole("link", { name: "Linked one" })).toHaveAttribute("data-menu-open", "true");
    expect(screen.getByRole("button", { name: "Closed one" })).not.toHaveAttribute("data-menu-open");
  });
});

describe("notify / Toasts", () => {
  it("shows a status note, an error as an alert, and lets each go after its time", () => {
    vi.useFakeTimers();
    try {
      render(<Toasts />);
      act(() => notify("Copied link"));
      expect(screen.getByRole("status")).toHaveTextContent("Copied link");
      act(() => notify("Couldn't merge: blocked by branch policy", { error: true }));
      const alert = screen.getByRole("alert");
      expect(alert).toHaveTextContent("Couldn't merge: blocked by branch policy");
      expect(alert).toHaveAttribute("data-error");
      act(() => {
        vi.advanceTimersByTime(4000);
      });
      expect(screen.queryByRole("status")).toBeNull();
      expect(screen.getByRole("alert")).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(4000);
      });
      expect(screen.queryByRole("alert")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("copyText", () => {
  it("falls back to the selection copy when the clipboard API refuses, and only reports a failure when both do", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("Write permission denied."));
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const exec = vi.fn().mockReturnValue(true);
    document.execCommand = exec;
    try {
      render(<Toasts />);
      await act(() => copyText("https://example.test/pr/1", "link"));
      expect(writeText).toHaveBeenCalledWith("https://example.test/pr/1");
      expect(exec).toHaveBeenCalledWith("copy");
      expect(screen.getByRole("status")).toHaveTextContent("Copied link");

      exec.mockReturnValue(false);
      await act(() => copyText("#7", "#7"));
      expect(screen.getByRole("alert")).toHaveTextContent("Couldn't copy #7");
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
