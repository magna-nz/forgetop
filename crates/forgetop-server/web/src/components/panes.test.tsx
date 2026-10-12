import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { mockFetch, renderWithClient } from "../test/util";
import { PipelineDetailProvider, usePipelineOpener } from "./PipelineDetail";
import { PrDetailProvider, usePrOpener } from "./PrDetail";
import { useWiOpener, WiDetailProvider } from "./WiDetail";

function Openers() {
  const openPr = usePrOpener();
  const openWi = useWiOpener();
  const openRun = usePipelineOpener();
  return (
    <>
      <button onClick={() => openPr({ conn: "c", repo: "acme/pay", id: "7" })}>open pr</button>
      <button onClick={() => openWi({ conn: "c", repo: "acme/pay", id: "42" })}>open wi</button>
      <button onClick={() => openRun({ conn: "c", repo: "acme/pay", runId: "r1" })}>open run</button>
    </>
  );
}

const regions = () => screen.queryAllByRole("region").map((r) => r.getAttribute("aria-label"));

describe("detail panes", () => {
  it("only one pane is open at a time, whichever kind opened last", async () => {
    mockFetch({ get: {} });
    renderWithClient(
      <PrDetailProvider>
        <WiDetailProvider>
          <PipelineDetailProvider>
            <Openers />
          </PipelineDetailProvider>
        </WiDetailProvider>
      </PrDetailProvider>,
    );
    await userEvent.click(screen.getByText("open pr"));
    await waitFor(() => expect(regions()).toEqual(["Pull request"]));
    await userEvent.click(screen.getByText("open wi"));
    await waitFor(() => expect(regions()).toEqual(["Work item"]));
    await userEvent.click(screen.getByText("open run"));
    await waitFor(() => expect(regions()).toEqual(["Pipeline run"]));
    await userEvent.click(screen.getByText("open pr"));
    await waitFor(() => expect(regions()).toEqual(["Pull request"]));
    // One Escape closes the one pane and nothing is left listening.
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(regions()).toEqual([]));
  });
});
