import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Button, IconButton, PaneHeader, PaneSlotContext, Row, Segmented, Select, SlideOver, StatusBadge, StatusGlyph, Tabs } from "./ui";

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
