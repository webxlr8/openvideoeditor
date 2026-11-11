import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { referenceSize } from "./ExportDialog";

async function openEditor() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBeGreaterThan(0));
}

describe("timeline viewer", () => {
  it("plays the composited timeline by default and switches to the clip source", async () => {
    await openEditor();
    const calls = window.__oveMock!.calls;
    await waitFor(() => expect(calls).toContain("openTimeline"));
    expect(screen.getByTestId("preview-timeline").getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByTestId("preview-source"));
    await waitFor(() => expect(calls.some((c) => c.startsWith("openMedia:"))).toBe(true));
    fireEvent.click(screen.getByTestId("preview-timeline"));
    await waitFor(() => expect(calls.filter((c) => c === "openTimeline").length).toBe(2));
  });

  it("drives the timeline transport", async () => {
    await openEditor();
    fireEvent.keyDown(window, { key: "End" });
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:59:29"));
  });

  it("maps aspect ratios to the engine's reference sizes", () => {
    expect(referenceSize("16:9")).toEqual([1920, 1080]);
    expect(referenceSize("9:16")).toEqual([1080, 1920]);
    expect(referenceSize("unknown")).toEqual([1920, 1080]);
  });
});

describe("export dialog", () => {
  it("exports with the chosen settings, shows progress and the result", async () => {
    await openEditor();
    fireEvent.click(screen.getByTestId("open-export"));
    const dialog = await screen.findByTestId("export-dialog");
    expect(dialog.textContent).toContain("1920×1080");
    fireEvent.click(screen.getByText("720p"));
    expect(dialog.textContent).toContain("1280×720");
    fireEvent.click(screen.getByText("60"));
    fireEvent.click(screen.getByTestId("export-start"));
    await screen.findByTestId("export-done", undefined, { timeout: 3000 });
    expect(window.__oveMock!.calls).toContain("exportProject:720p60");
    expect(screen.getByTestId("export-done").textContent).toContain(".mp4");
    fireEvent.click(screen.getByTestId("export-reveal"));
    expect(window.__oveMock!.calls.some((c) => c.startsWith("revealPath:"))).toBe(true);
  });

  it("cancels a running export", async () => {
    await openEditor();
    fireEvent.click(screen.getByTestId("open-export"));
    fireEvent.click(await screen.findByTestId("export-start"));
    fireEvent.click(await screen.findByTestId("export-cancel"));
    const err = await screen.findByTestId("export-error");
    expect(err.textContent).toContain("cancelled");
    expect(window.__oveMock!.calls).toContain("cancelExport");
  });
});
