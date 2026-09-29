import { describe, it, expect } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Pipelines } from "./Pipelines";
import { pipelineLocation } from "./PipelineScope";
import { renderWithClient, mockFetch } from "../test/util";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const connection: any = {
  id: "ado",
  provider: "AzureDevOps",
  display_name: "ADO",
  base_url: null,
  organization: "acme",
  project: null,
  repository: null,
  username: null,
  repo_scope: null,
  has_token: true,
  sections: ["pipelines"],
};

const defs = [
  { id: "1", name: "MainLine", repository: "Tilt", path: "\\Releases\\Core", url: null },
  { id: "2", name: "Tests", repository: "Tilt", path: "\\", url: null },
  { id: "3", name: "Nightly", repository: "Ops", path: "\\", url: null },
];

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const run = (definition_id: string): any => ({
  connection_id: "ado",
  connection: "ADO",
  provider: "AzureDevOps",
  definition_name: definition_id === "1" ? "MainLine" : "Tests",
  approvals: [],
  run: {
    id: `r${definition_id}`,
    definition_id,
    repository: "Tilt",
    name: null,
    number: 1,
    status: "Succeeded",
    branch: "main",
    commit_sha: null,
    title: null,
    started_at: "2026-09-01T00:00:00Z",
    finished_at: "2026-09-01T00:05:00Z",
    triggered_by: null,
    stages: [],
    url: null,
  },
});

/** GET routes match by URL substring in insertion order, so each longer endpoint is listed before
 *  the shorter one it would otherwise match (`/api/pipelines/definitions` before `/api/pipelines`). */
const routes = (selection: { all: boolean; selected: string[] }, runs: unknown[]) => ({
  "/api/pipelines/definitions": { definitions: defs, ...selection },
  "/api/pipelines": runs,
  "/api/connections/repositories": { repositories: [], truncated: false },
  "/api/connections": [connection],
});

describe("pipeline selection", () => {
  it("starts opt-in: nothing selected reads as a choice to make, not as 'no runs'", async () => {
    mockFetch({ get: routes({ all: false, selected: [] }, []) });
    renderWithClient(<Pipelines />);
    expect(await screen.findByText("No pipelines selected")).toBeInTheDocument();
    expect(screen.queryByText("No pipeline runs")).not.toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Pipelines · 0 of 3" })).toBeInTheDocument();
  });

  it("counts an explicit selection, and 'all' as every pipeline", async () => {
    mockFetch({ get: routes({ all: false, selected: ["1", "gone"] }, [run("1")]) });
    const first = renderWithClient(<Pipelines />);
    // A saved id discovery no longer knows isn't counted as fetched.
    expect(await screen.findByRole("button", { name: "Pipelines · 1 of 3" })).toBeInTheDocument();
    first.unmount();

    mockFetch({ get: routes({ all: true, selected: [] }, [run("1")]) });
    renderWithClient(<Pipelines />);
    expect(await screen.findByRole("button", { name: "Pipelines · 3 of 3" })).toBeInTheDocument();
  });

  it("saves ticks at once, and saves everything ticked as 'all'", async () => {
    const { posts } = mockFetch({ get: routes({ all: false, selected: ["1", "2"] }, [run("1"), run("2")]) });
    renderWithClient(<Pipelines />);
    await userEvent.click(await screen.findByRole("button", { name: "Pipelines · 2 of 3" }));

    await userEvent.click(await screen.findByRole("checkbox", { name: "Tests" }));
    expect(posts.at(-1)?.url).toContain("/api/pipelines/selection");
    expect(posts.at(-1)?.body).toEqual({ id: "ado", all: false, ids: ["1"] });
    expect(screen.getByRole("button", { name: "Pipelines · 1 of 3" })).toBeInTheDocument();

    // "All" ticks everything shown, which is every pipeline — saved as "all" so a pipeline
    // created later is fetched too.
    await userEvent.click(screen.getByRole("button", { name: "All" }));
    expect(posts.at(-1)?.body).toEqual({ id: "ado", all: true, ids: [] });

    // "None" clears — an explicit empty choice.
    await userEvent.click(screen.getByRole("button", { name: "None" }));
    expect(posts.at(-1)?.body).toEqual({ id: "ado", all: false, ids: [] });
  });

  it("'All' with a search ticks only what the search shows", async () => {
    const { posts } = mockFetch({ get: routes({ all: false, selected: [] }, []) });
    renderWithClient(<Pipelines />);
    await userEvent.click(await screen.findByRole("button", { name: "Pipelines · 0 of 3" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Search ADO pipelines" }), "tilt");
    await userEvent.click(screen.getByRole("button", { name: "All" }));
    expect((posts.at(-1)?.body as { ids: string[] }).ids.sort()).toEqual(["1", "2"]);
  });

  it("says where a pipeline lives: project and folder, never a bare root folder", () => {
    expect(pipelineLocation(defs[0])).toBe("Tilt \\Releases\\Core");
    expect(pipelineLocation(defs[1])).toBe("Tilt");
    expect(pipelineLocation({ id: "x", name: "ci", repository: "acme/pay", path: ".github/workflows/ci.yml" })).toBe("acme/pay");
  });
});
