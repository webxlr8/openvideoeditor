import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";

const clips = () => screen.queryAllByTestId("clip");

async function editorWithCameraAndRecorder() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(clips().length).toBe(1));
  await act(async () => window.__oveMock!.routePaths(["/tmp/Recorder.wav"]));
  await waitFor(() => expect(clips().length).toBe(2));
}

describe("audio sync", () => {
  it("synchronises the selection against the clicked clip and shows a summary", async () => {
    await editorWithCameraAndRecorder();
    const camera = clips().find((c) => c.textContent?.includes("Sample clip.mov"))!;
    const recorder = clips().find((c) => c.textContent?.includes("Recorder.wav"))!;
    const before = recorder.getAttribute("title");
    fireEvent.pointerDown(camera, { button: 0, clientX: 300 });
    fireEvent.pointerUp(window);
    fireEvent.pointerDown(recorder, { button: 0, clientX: 300, ctrlKey: true });
    fireEvent.pointerUp(window);
    fireEvent.contextMenu(camera, { clientX: 300, clientY: 600 });
    fireEvent.click(await screen.findByTestId("menu-sync"));
    const summary = await screen.findByTestId("sync-summary");
    expect(summary.textContent).toContain("Synchronised 1 of 1 item");
    await waitFor(() =>
      expect(clips().find((c) => c.textContent?.includes("Recorder.wav"))!.getAttribute("title")).not.toBe(before),
    );
    expect(clips().find((c) => c.textContent?.includes("Recorder.wav"))!.getAttribute("title")).toContain("00:01.50");
    // One revision: one undo puts the recorder back.
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() =>
      expect(clips().find((c) => c.textContent?.includes("Recorder.wav"))!.getAttribute("title")).toBe(before),
    );
    fireEvent.click(screen.getByTestId("sync-summary-close"));
    expect(screen.queryByTestId("sync-summary")).toBeNull();
  });

  it("offers no sync for a single clip", async () => {
    await editorWithCameraAndRecorder();
    const camera = clips().find((c) => c.textContent?.includes("Sample clip.mov"))!;
    fireEvent.pointerDown(camera, { button: 0, clientX: 300 });
    fireEvent.pointerUp(window);
    fireEvent.contextMenu(camera, { clientX: 300, clientY: 600 });
    await screen.findByTestId("menu-delete");
    expect(screen.queryByTestId("menu-sync")).toBeNull();
  });
});
