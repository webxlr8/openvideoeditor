import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { fitBox } from "./Viewer";

async function openEditorWithMedia() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
}

describe("App", () => {
  it("starts on Home and opens the editor with media", async () => {
    await openEditorWithMedia();
    expect(screen.getAllByText("Sample clip.mov").length).toBeGreaterThan(0);
    expect(screen.getByTestId("timecode").textContent).toBe("00:00:00:00");
  });

  it("drives the transport from buttons and keys", async () => {
    await openEditorWithMedia();
    for (let i = 0; i < 10; i++) fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:00:10"));
    fireEvent.keyDown(window, { key: "End" });
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:59:29"));
    fireEvent.click(screen.getByTestId("loop"));
    await waitFor(() => expect(screen.getByTestId("loop").getAttribute("aria-pressed")).toBe("true"));
  });

  it("seeks by typed timecode", async () => {
    await openEditorWithMedia();
    fireEvent.click(screen.getByTestId("timecode"));
    const input = screen.getByTestId("timecode-input");
    fireEvent.change(input, { target: { value: "00:00:12:15" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:12:15"));
  });

  it("shows a plain-language error for an unsupported routed file and keeps state", async () => {
    await openEditorWithMedia();
    await act(async () => window.__oveMock!.routePaths(["/tmp/notes.txt"]));
    const toast = await screen.findByTestId("toast");
    expect(toast.textContent).toContain("notes.txt");
    expect(toast.getAttribute("data-code")).toBe("unsupported");
    expect(screen.getAllByText("Sample clip.mov").length).toBeGreaterThan(0);
  });

  it("hides the viewer when the timeline is expanded", async () => {
    await openEditorWithMedia();
    expect(screen.queryByTestId("viewer-area")).toBeTruthy();
    fireEvent.click(screen.getByTestId("toggle-viewer"));
    expect(screen.queryByTestId("viewer-area")).toBeNull();
    await waitFor(() => expect(window.__oveMock!.lastRect?.visible).toBe(false));
  });

  it("toggles panel, timeline and chat", async () => {
    await openEditorWithMedia();
    fireEvent.click(screen.getByTestId("collapse-panel"));
    expect(screen.queryByTestId("side-panel")).toBeNull();
    fireEvent.click(screen.getByTestId("toggle-timeline"));
    expect(screen.queryByTestId("timeline")).toBeNull();
    expect(screen.getByTestId("scrub-row")).toBeTruthy();
    fireEvent.click(screen.getByTestId("close-chat"));
    expect(screen.queryByTestId("chat")).toBeNull();
  });
});

describe("fitBox", () => {
  it("fits 16:9 into the padded area, centred", () => {
    expect(fitBox(1000, 600, 16 / 9, 1)).toEqual({ left: 24, top: 33, width: 952, height: 535 });
    const b = fitBox(500, 1000, 16 / 9, 1);
    expect(b.width).toBe(452);
    expect(b.left).toBe(24);
  });

  it("imports into a project, edits the timeline and undoes", async () => {
    await openEditorWithMedia();
    await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
    expect(screen.getByTestId("track-Main").textContent).toContain("Sample clip.mov");
    // The imported clip is selected, so its properties show; closing them returns to the media panel.
    expect(screen.getByTestId("inspector")).toBeTruthy();
    fireEvent.click(screen.getByTestId("close-inspector"));
    expect(screen.getAllByTestId("media-card").length).toBe(1);
    // The imported clip is selected and previewed; Delete removes it, undo brings it back.
    fireEvent.keyDown(window, { key: "Delete" });
    await waitFor(() => expect(screen.queryAllByTestId("clip").length).toBe(0));
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
    fireEvent.keyDown(window, { key: "z", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(screen.queryAllByTestId("clip").length).toBe(0));
  });

  it("lists the project on Home after going back", async () => {
    await openEditorWithMedia();
    fireEvent.click(screen.getByTestId("go-home"));
    await screen.findByTestId("home");
    await waitFor(() => expect(screen.getAllByTestId("project-card").length).toBeGreaterThan(0));
    fireEvent.click(screen.getAllByTestId("project-card")[0]);
    await screen.findByTestId("editor");
    expect(screen.getByTestId("project-name").textContent).toBeTruthy();
  });
});
