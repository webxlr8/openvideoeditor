import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";

async function view() {
  const v = await commands.projectView();
  if (v.status !== "ok" || !v.data) throw new Error("no project");
  return v.data;
}

async function editorWithHistory() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  fireEvent.click(screen.getByTestId("history"));
  return screen.findByTestId("history-panel");
}

describe("versions in the History panel", () => {
  it("saves a version, compares it with now and restores it as one undoable revision", async () => {
    const panel = await editorWithHistory();
    const clip = (await view()).items[0];
    fireEvent.change(within(panel).getByTestId("version-name"), { target: { value: "client v2" } });
    fireEvent.click(within(panel).getByTestId("version-save"));
    await waitFor(() => expect(within(panel).getAllByTestId("version-row")).toHaveLength(1));
    expect(within(panel).getByTestId("version-row").textContent).toContain("client v2");

    // Trim the clip, then compare: the clip is listed as trimmed.
    await act(async () => {
      await commands.updateItem(clip.id, JSON.stringify({ endTime: (clip.end ?? 0) - 1 }), [], "trim");
    });
    fireEvent.click(within(panel).getByTestId("version-compare"));
    const change = await within(panel).findByTestId("version-change");
    expect(change.textContent).toContain(`trimmed item ${clip.id}`);

    const before = (await view()).rev ?? 0;
    fireEvent.click(within(panel).getByTestId("version-restore"));
    await waitFor(async () => expect((await view()).items[0].end).toBe(clip.end));
    expect((await view()).rev).toBe(before + 1);
    await act(async () => {
      await commands.undo();
    });
    expect((await view()).items[0].end).toBe((clip.end ?? 0) - 1);
  });

  it("refuses a duplicate version name", async () => {
    const panel = await editorWithHistory();
    for (let n = 0; n < 2; n++) {
      fireEvent.change(within(panel).getByTestId("version-name"), { target: { value: "v1" } });
      fireEvent.click(within(panel).getByTestId("version-save"));
    }
    await waitFor(() => expect(within(panel).getAllByTestId("version-row")).toHaveLength(1));
    expect((await commands.versionSave("v1", null)).status).toBe("error");
  });
});
