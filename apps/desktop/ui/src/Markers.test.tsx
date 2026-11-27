import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import { markerChanges } from "./MarkerLane";

async function editor() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
}

async function rev(): Promise<number> {
  const v = await commands.projectView();
  if (v.status !== "ok" || !v.data) throw new Error("no project");
  return v.data.rev ?? 0;
}

const lane = () => screen.getByTestId("marker-lane");
const shown = () => within(lane()).queryAllByTestId("marker");

describe("markers", () => {
  it("M adds a point marker at the playhead in one revision, and undo removes it", async () => {
    await editor();
    expect(shown()).toHaveLength(0);
    const before = await rev();
    fireEvent.keyDown(window, { key: "m" });
    await waitFor(() => expect(shown()).toHaveLength(1));
    expect(shown()[0].dataset.status).toBe("open");
    expect(await rev()).toBe(before + 1);
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(shown()).toHaveLength(0));
  });

  it("edits a marker in the popover as one revision", async () => {
    await editor();
    await commands.markerAdd(1, JSON.stringify({ name: "Tighten", assignee: "agent" }));
    await waitFor(() => expect(shown()).toHaveLength(1));
    fireEvent.click(shown()[0]);
    const pop = await screen.findByTestId("marker-popover");
    expect(within(pop).getByTestId<HTMLInputElement>("marker-name").value).toBe("Tighten");
    const before = await rev();
    fireEvent.change(within(pop).getByTestId("marker-name"), { target: { value: "Tighten the pause" } });
    fireEvent.change(within(pop).getByTestId("marker-note"), { target: { value: "Cut 10 frames" } });
    fireEvent.change(within(pop).getByTestId("marker-color"), { target: { value: "red" } });
    fireEvent.change(within(pop).getByTestId("marker-status"), { target: { value: "resolved" } });
    fireEvent.change(within(pop).getByTestId("marker-assignee"), { target: { value: "" } });
    fireEvent.click(within(pop).getByTestId("marker-save"));
    await waitFor(() => expect(screen.queryByTestId("marker-popover")).toBeNull());
    await waitFor(() => expect(shown()[0].dataset.status).toBe("resolved"));
    expect(shown()[0].textContent).toContain("Tighten the pause");
    expect(await rev()).toBe(before + 1);
    const v = await commands.projectView();
    const m = v.status === "ok" ? v.data?.markers?.[0] : undefined;
    expect(m).toMatchObject({ name: "Tighten the pause", note: "Cut 10 frames", color: "red", status: "resolved", assignee: null });
  });

  it("filters the lane by status without changing the document", async () => {
    await editor();
    await commands.markerAdd(1, JSON.stringify({ status: "open" }));
    await commands.markerAdd(2, JSON.stringify({ status: "review" }));
    await commands.markerAdd(3, JSON.stringify({ status: "resolved" }));
    await waitFor(() => expect(shown()).toHaveLength(3));
    const before = await rev();
    fireEvent.change(screen.getByTestId("marker-filter"), { target: { value: "review" } });
    expect(shown()).toHaveLength(1);
    expect(shown()[0].dataset.status).toBe("review");
    fireEvent.change(screen.getByTestId("marker-filter"), { target: { value: "all" } });
    expect(shown()).toHaveLength(3);
    expect(await rev()).toBe(before);
  });

  it("deletes a marker from the popover in one revision", async () => {
    await editor();
    await commands.markerAdd(4, JSON.stringify({ dur: 2, name: "Range" }));
    await waitFor(() => expect(shown()).toHaveLength(1));
    expect(shown()[0].className).toContain("marker-range");
    const before = await rev();
    fireEvent.click(shown()[0]);
    fireEvent.click(await screen.findByTestId("marker-delete"));
    await waitFor(() => expect(shown()).toHaveLength(0));
    expect(await rev()).toBe(before + 1);
  });

  it("a save with nothing changed makes no revision", () => {
    const m = { id: "m", at: 1, dur: 0, status: "open", name: "A", color: null, note: null, assignee: null };
    expect(markerChanges(m, { name: "A", note: "", color: "", status: "open", assignee: "" })).toEqual({ set: {}, unset: [] });
    expect(markerChanges(m, { name: "", note: "x", color: "", status: "review", assignee: "" })).toEqual({
      set: { note: "x", status: "review" },
      unset: ["name"],
    });
  });
});
