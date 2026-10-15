import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";

describe("public demo shell", () => {
  it("identifies itself as simulated and resets browser-memory actions", () => {
    render(<App />);

    expect(screen.getByText("Interactive public demo.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Pull Requests/ }));
    fireEvent.click(screen.getByRole("button", { name: /Refactor the webhook retry queue/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Add a pull request comment" }), { target: { value: "A browser-only comment" } });
    fireEvent.click(screen.getByRole("button", { name: "Post comment" }));
    expect(screen.getByText("A browser-only comment")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Reset demo" }));
    expect(screen.getByRole("status")).toHaveTextContent("No actions yet");
    fireEvent.click(screen.getByRole("button", { name: /Pull Requests/ }));
    fireEvent.click(screen.getByRole("button", { name: /Refactor the webhook retry queue/ }));
    expect(screen.queryByText("A browser-only comment")).not.toBeInTheDocument();
  });

  it("opens notification targets and marks them as simulated reads", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    fireEvent.click(screen.getByRole("button", { name: /Pipeline running/ }));
    expect(screen.getByRole("dialog", { name: /infrastructure \/ plan/ })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Notification marked read (simulated)");
  });

  it("opens the command palette with Cmd/Ctrl+K", () => {
    render(<App />);

    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Find a command" })).toHaveFocus();
  });
});
