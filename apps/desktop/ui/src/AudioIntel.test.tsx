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

describe("snap to beats", () => {
  // The clip title ends with its end time as mm:ss.cc.
  const endOf = (c: HTMLElement) => {
    const m = /–(\d+):(\d+)\.(\d+)$/.exec(c.getAttribute("title") ?? "")!;
    return Number(m[1]) * 60 + Number(m[2]) + Number(m[3]) / 100;
  };

  async function editorWithMusic() {
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    await waitFor(() => expect(clips().length).toBe(1));
    await act(async () => window.__oveMock!.routePaths(["/tmp/Music.wav"]));
    await waitFor(() => expect(clips().length).toBe(2));
  }

  /** Drags the camera clip's end edge (at 60 s) to timeline second `to`. */
  async function dragCameraEnd(to: number) {
    const camera = clips().find((c) => c.textContent?.includes("Sample clip.mov"))!;
    const pps = parseFloat(camera.style.width) / 60;
    const dx = (to - 60) * pps;
    fireEvent.pointerDown(camera, { button: 0, clientX: 300 });
    fireEvent.pointerUp(window);
    const handle = await screen.findByTestId("trim-end");
    const before = endOf(camera);
    fireEvent.pointerDown(handle, { button: 0, clientX: 500 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 500 + dx }));
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 500 + dx }));
    });
    await waitFor(() => expect(endOf(clips().find((c) => c.textContent?.includes("Sample clip.mov"))!)).not.toBe(before));
    return endOf(clips().find((c) => c.textContent?.includes("Sample clip.mov"))!);
  }

  it("draws beat marks and lands a dragged edge on a downbeat", async () => {
    await editorWithMusic();
    expect(screen.queryAllByTestId("beat-mark").length).toBe(0);
    fireEvent.click(screen.getByTestId("snap-beats"));
    await waitFor(() => expect(screen.queryAllByTestId("downbeat-mark").length).toBeGreaterThan(0));
    expect(screen.queryAllByTestId("beat-mark").length).toBeGreaterThan(0);
    const end = await dragCameraEnd(56.35);
    // 56.35 s: the beat at 56.5 s is closer, but the downbeat at 56 s is in reach and wins.
    expect(end).toBe(56);
  });

  it("draws no marks and does not snap to beats when the option is off", async () => {
    await editorWithMusic();
    expect(screen.queryAllByTestId("beat-mark").length).toBe(0);
    const end = await dragCameraEnd(56.35);
    expect(end).not.toBe(56);
    expect(end).not.toBe(56.5);
    expect(screen.queryAllByTestId("downbeat-mark").length).toBe(0);
  });
});
