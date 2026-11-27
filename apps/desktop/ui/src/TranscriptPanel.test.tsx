import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type { MockWord } from "./bridge";
import { sentences } from "./TranscriptPanel";

const word = (
  _n: number,
  text: string,
  start: number,
  filler = false,
  contextFiller = false,
): MockWord => ({
  id: `m@${Math.round(start * 1000)}`,
  text,
  start,
  end: start + 0.4,
  clipId: "c1",
  mediaId: "m",
  sourceStart: start,
  sourceEnd: start + 0.4,
  filler,
  contextFiller,
});

function talk() {
  window.__oveMock!.transcript = {
    words: [
      word(0, "Hello", 0.2),
      word(1, "um", 0.8, true),
      word(2, "everyone.", 1.3),
      word(3, "We", 4.2),
      word(4, "like", 4.7, false, true),
      word(5, "café", 5.2),
      word(6, "editing.", 5.7),
    ],
    pauses: [{ start: 2.1, end: 3.9 }],
  };
}

async function panel() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  fireEvent.click(screen.getByTestId("rail-transcript"));
  await screen.findByTestId("word-m@200");
}

const struck = () =>
  Array.from(document.querySelectorAll(".transcript-word.struck")).map((e) =>
    e.textContent?.trim(),
  );
const texts = () =>
  Array.from(document.querySelectorAll(".transcript-word")).map((e) =>
    e.textContent?.trim(),
  );

describe("transcript panel", () => {
  it("groups words into sentences", () => {
    const s = sentences(
      [word(0, "A", 0), word(1, "b.", 0.5), word(2, "C", 3)].map((w) => ({
        ...w,
        clipId: "c",
      })),
    );
    expect(s.map((x) => x.map((w) => w.text).join(" "))).toEqual(["A b.", "C"]);
  });

  it("seeks on click, strikes through a Shift-selected run, previews the cut on Delete and applies it as one undo step", async () => {
    talk();
    await panel();
    expect(texts()).toEqual([
      "Hello",
      "um",
      "everyone.",
      "We",
      "like",
      "café",
      "editing.",
    ]);
    // Click: seek there and select the word.
    fireEvent.click(screen.getByTestId("word-m@4700"));
    await waitFor(() =>
      expect(window.__oveMock!.calls).toContain("transport:seekFrame"),
    );
    // Shift-click: the run from the first word.
    fireEvent.click(screen.getByTestId("word-m@5200"), { shiftKey: true });
    expect(struck()).toEqual(["like", "café"]);
    // Delete: a dry run first, nothing changes yet.
    fireEvent.keyDown(screen.getByTestId("transcript-panel"), {
      key: "Delete",
    });
    const preview = await screen.findByTestId("transcript-preview");
    expect(preview.textContent).toContain("remove 2 words");
    expect(screen.getByTestId("transcript-diff").textContent).toContain(
      "-like café",
    );
    expect(window.__oveMock!.calls).toContain("transcriptQuery:remove_words");
    expect(texts()).toContain("like");
    // Cut: one revision; the panel reads the transcript again.
    fireEvent.click(screen.getByTestId("transcript-apply"));
    await waitFor(() =>
      expect(texts()).toEqual(["Hello", "um", "everyone.", "We", "editing."]),
    );
    expect(screen.getByTestId("transcript-message").textContent).toContain(
      "Undo puts it back",
    );
    // One undo step brings both words back.
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(texts()).toContain("like"));
  });

  it("Ctrl-click adds and removes single words; Escape clears the selection", async () => {
    talk();
    await panel();
    fireEvent.click(screen.getByTestId("word-m@200"), { ctrlKey: true });
    fireEvent.click(screen.getByTestId("word-m@5700"), { ctrlKey: true });
    expect(struck()).toEqual(["Hello", "editing."]);
    fireEvent.click(screen.getByTestId("word-m@200"), { ctrlKey: true });
    expect(struck()).toEqual(["editing."]);
    fireEvent.keyDown(screen.getByTestId("transcript-panel"), {
      key: "Escape",
    });
    expect(struck()).toEqual([]);
  });

  it("Remove pauses shows the ranges, then cuts them", async () => {
    talk();
    await panel();
    fireEvent.click(screen.getByTestId("transcript-pauses"));
    const ranges = await screen.findByTestId("transcript-ranges");
    expect(ranges.textContent).toContain("0:02.1–0:03.9");
    // The previewed range is drawn over the clips on the timeline.
    expect(screen.getAllByTestId("cut-preview").length).toBe(1);
    fireEvent.click(screen.getByTestId("transcript-apply"));
    await waitFor(() =>
      expect(window.__oveMock!.calls).toContain("transcriptCut:remove_silence"),
    );
    await waitFor(() =>
      expect(screen.queryAllByTestId("cut-preview").length).toBe(0),
    );
    await waitFor(() =>
      expect(screen.getByTestId("transcript-message").textContent).toContain(
        "remove dead air (1 cuts, 1.8 s)",
      ),
    );
    expect(texts()).toEqual([
      "Hello",
      "um",
      "everyone.",
      "We",
      "like",
      "café",
      "editing.",
    ]);
  });

  it("Remove fillers lists candidates for review: hesitations checked, context words not", async () => {
    talk();
    await panel();
    fireEvent.click(screen.getByTestId("transcript-fillers"));
    await screen.findByTestId("transcript-filler-list");
    expect(
      screen.getByTestId<HTMLInputElement>("transcript-filler-m@800").checked,
    ).toBe(true);
    expect(
      screen.getByTestId<HTMLInputElement>("transcript-filler-m@4700").checked,
    ).toBe(false);
    expect(screen.getByTestId("transcript-filler-list").textContent).toContain(
      "depends on context",
    );
    // Nothing is cut until the person confirms the preview.
    const cuts = () =>
      window.__oveMock!.calls.filter((c) => c === "transcriptCut:remove_words")
        .length;
    const before = cuts();
    fireEvent.click(screen.getByTestId("transcript-fillers-cut"));
    await screen.findByTestId("transcript-preview");
    expect(cuts()).toBe(before);
    fireEvent.click(screen.getByTestId("transcript-apply"));
    await waitFor(() => expect(texts()).not.toContain("um"));
    expect(texts()).toContain("like");
  });

  it("search finds a phrase ignoring case and accents and seeks to it", async () => {
    talk();
    await panel();
    fireEvent.change(screen.getByTestId("transcript-search"), {
      target: { value: "CAFE editing" },
    });
    fireEvent.submit(screen.getByTestId("transcript-search"));
    const hit = await screen.findByTestId("transcript-hit-m@5200");
    expect(hit.textContent).toContain("café editing.");
    expect(document.querySelectorAll(".transcript-word.hit").length).toBe(2);
    fireEvent.click(hit);
    await waitFor(() =>
      expect(window.__oveMock!.calls).toContain("transport:seekFrame"),
    );
    fireEvent.change(screen.getByTestId("transcript-search"), {
      target: { value: "nothing" },
    });
    fireEvent.submit(screen.getByTestId("transcript-search"));
    await waitFor(() =>
      expect(screen.getByTestId("transcript-hits").textContent).toContain(
        "No match.",
      ),
    );
  });

  it("says what to do when nothing is transcribed", async () => {
    window.__oveMock!.transcript = { words: [], pauses: [] };
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    fireEvent.click(screen.getByTestId("rail-transcript"));
    expect(
      (await screen.findByTestId("transcript-empty")).textContent,
    ).toContain("no clip with speech");
  });
});
