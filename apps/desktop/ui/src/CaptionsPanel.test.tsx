import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { retimeSegment } from "./CaptionsPanelStyles";

async function captionsPanel() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  fireEvent.click(screen.getByTestId("rail-captions"));
  await screen.findByTestId("caption-auto");
}

describe("captions panel", () => {
  it("generates captions from the Main track, downloading the model first, as one undo step", async () => {
    window.__oveMock!.captions = { installed: [], stepMs: 5 };
    await captionsPanel();
    // The default model is not on disk yet: the panel says it downloads the first time.
    expect((await screen.findByTestId("caption-model-note")).textContent).toContain("147 MB");
    fireEvent.change(screen.getByTestId("caption-language"), { target: { value: "en" } });
    fireEvent.change(screen.getByTestId("caption-new-style"), { target: { value: "word-pop" } });
    fireEvent.click(screen.getByTestId("caption-generate"));
    await screen.findByTestId("caption-progress");
    await waitFor(() => expect(screen.getByTestId("caption-progress").textContent).toContain("Downloading"));
    await waitFor(() => expect(screen.getByTestId("caption-result").textContent).toContain("Added 3 captions (en)"), { timeout: 3000 });
    expect(window.__oveMock!.calls).toContain("autoCaption:base:en:0");
    expect(screen.getByTestId("track-Captions").textContent).toContain("Welcome back to the channel");
    expect(screen.getAllByTestId("caption-line").length).toBe(3);
    // The model is installed now: no download note.
    await waitFor(() => expect(screen.queryByTestId("caption-model-note")).toBeNull());
    // One revision: a single undo removes every caption.
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(screen.queryAllByTestId("caption-line").length).toBe(0));
  });

  it("cancels a running job", async () => {
    window.__oveMock!.captions = { installed: [], stepMs: 40 };
    await captionsPanel();
    fireEvent.click(screen.getByTestId("caption-generate"));
    fireEvent.click(await screen.findByTestId("caption-cancel"));
    await waitFor(() => expect(screen.getByTestId("caption-result").textContent).toBe("Stopped."));
    expect(screen.queryAllByTestId("caption-line").length).toBe(0);
  });

  it("shows the 21 styles and applies one to all captions", async () => {
    window.__oveMock!.captions = { installed: ["base"], stepMs: 1 };
    await captionsPanel();
    await waitFor(() => expect(document.querySelectorAll("[data-testid^='caption-style-']").length).toBe(22)); // + "All captions"
    fireEvent.click(screen.getByTestId("caption-generate"));
    await waitFor(() => expect(screen.getAllByTestId("caption-line").length).toBe(3), { timeout: 3000 });
    fireEvent.click(screen.getByTestId("caption-style-neo-brutal"));
    await waitFor(() => expect(window.__oveMock!.calls).toContain("applyCaptionStyle:neo-brutal:0"));
    expect(within(screen.getByTestId("caption-style-neo-brutal")).getByTestId("caption-sample-neo-brutal").textContent).toContain("video");
  });

  it("edits a caption's words in the transcript", async () => {
    window.__oveMock!.captions = { installed: ["base"], stepMs: 1 };
    await captionsPanel();
    fireEvent.click(screen.getByTestId("caption-generate"));
    await waitFor(() => expect(screen.getAllByTestId("caption-line").length).toBe(3), { timeout: 3000 });
    const box = screen.getAllByTestId("caption-line")[0].querySelector("textarea")!;
    fireEvent.change(box, { target: { value: "Welcome back to our channel" } });
    fireEvent.keyDown(box, { key: "Enter" });
    fireEvent.blur(box);
    await waitFor(() => expect(screen.getByTestId("track-Captions").textContent).toContain("Welcome back to our channel"));
  });
});

describe("retimeSegment", () => {
  const seg = {
    id: "s1",
    text: "hello there world",
    startTime: 0,
    endTime: 3,
    wordTimings: [
      { word: "hello", start: 0, end: 1 },
      { word: "there", start: 1, end: 2 },
      { word: "world", start: 2, end: 3 },
    ],
  };
  it("keeps timings for a one-to-one fix", () => {
    const s = retimeSegment(seg, "Hello their world");
    expect(s.wordTimings?.map((w) => [w.word, w.start])).toEqual([
      ["Hello", 0],
      ["their", 1],
      ["world", 2],
    ]);
  });
  it("spreads timings when words are added or removed", () => {
    const s = retimeSegment(seg, "hi world");
    expect(s.wordTimings?.length).toBe(2);
    expect(s.wordTimings?.[0].start).toBe(0);
    expect(s.wordTimings?.[1].end).toBe(3);
  });
});
