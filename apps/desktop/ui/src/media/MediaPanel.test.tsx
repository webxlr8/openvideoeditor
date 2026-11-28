import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";
import { CLIP_DRAG_TYPE, DEBOUNCE_MS, pillText } from "./MediaPanel";

const moment = {
  media: "m1",
  name: "harbour",
  shot: [10, 30],
  best: 14,
  in: 12,
  out: 16,
  confidence: 0.83,
  clip: "<clip start={0} end={4} in={12} src=\"harbour\" />",
};
const spoken = {
  media: "m1",
  name: "harbour",
  text: "the price is",
  context: "So the price is right.",
  in: 2.25,
  out: 3.65,
  clip: "<clip start={0} end={1.4} in={2.25} src=\"harbour\" />",
};

function results(over: Record<string, unknown> = {}) {
  window.__oveMock!.mediaSearch.result = {
    moments: [moment],
    spoken: [spoken],
    timelines: [{ item: "i1", type: "text", start: 1, end: 3, name: "Harbour title", text: "" }],
    files: [{ media: "m1", name: "harbour", folder: "Selects", tags: ["boat"] }],
    index: { visual: "ready", indexed: 1, total: 1, transcribed: 1, untranscribed: 0 },
    ...over,
  };
}

/** The open project's first media item (the results point at it). */
let first = { id: "m1", name: "harbour" };

async function panel() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  const v = await commands.projectView();
  const m = v.status === "ok" ? v.data?.media[0] : undefined;
  if (m) first = { id: m.id, name: m.name };
  for (const list of ["moments", "spoken", "files"] as const) {
    const hits = window.__oveMock!.mediaSearch.result[list] as { media: string; name: string }[] | undefined;
    for (const h of hits ?? []) Object.assign(h, { media: first.id, name: first.name });
  }
  fireEvent.click(screen.getByTestId("rail-media"));
  return screen.findByTestId("media-panel");
}

async function search(q: string) {
  fireEvent.change(screen.getByTestId("media-search"), { target: { value: q } });
  await act(async () => new Promise((r) => setTimeout(r, DEBOUNCE_MS + 20)));
  await screen.findByTestId("media-results");
}

const calls = () => window.__oveMock!.calls.filter((c) => c.startsWith("mediaSearch"));

describe("media panel", () => {
  it("searches after typing stops and shows Moments, Spoken, Timelines and Files", async () => {
    results();
    await panel();
    const before = calls().length;
    await search("harbour");
    expect(calls().length).toBe(before + 1);
    expect(await screen.findByTestId("moment")).toBeTruthy();
    expect(screen.getByTestId("spoken").textContent).toContain("the price is");
    expect(screen.getByTestId("timeline-hit").textContent).toContain("Harbour title");
    expect(screen.getByTestId("file-hit").textContent).toContain("Selects · boat");
    // A scope chip limits the sections and searches that scope.
    fireEvent.click(screen.getByTestId("scope-spoken"));
    await waitFor(() => expect(calls().at(-1)).toBe("mediaSearch:spoken"));
    expect(screen.queryByTestId("section-moments")).toBeNull();
    expect(screen.getByTestId("section-spoken")).toBeTruthy();
    // Clearing the field shows the media again.
    fireEvent.change(screen.getByTestId("media-search"), { target: { value: "" } });
    await waitFor(() => expect(screen.queryByTestId("media-results")).toBeNull());
  });

  it("says when nothing looks like the query, and offers the install when the model is missing", async () => {
    results({ moments: [], index: { visual: "ready", indexed: 2, total: 2, untranscribed: 3 } });
    window.__oveMock!.mediaSearch.status = { ...window.__oveMock!.mediaSearch.status, modelInstalled: false, downloadBytes: 755_234_909, indexed: 0, total: 2, running: 0 };
    await panel();
    await search("volcano");
    expect((await screen.findByTestId("moments-empty")).textContent).toBe("No visual matches.");
    expect(screen.getByTestId("spoken")).toBeTruthy();
    expect(screen.getByTestId("untranscribed").textContent).toContain("3 files are not");

    results({ moments: [], index: { visual: "notInstalled", downloadBytes: 755_234_909, indexed: 0, total: 2 } });
    fireEvent.change(screen.getByTestId("media-search"), { target: { value: "volcano erupting" } });
    expect((await screen.findByTestId("moments-not-installed")).textContent).toContain("0.76 GB");
    // Without the model, Spoken, Timelines and Files still list their results.
    expect(screen.getByTestId("spoken").textContent).toContain("the price is");
    expect(screen.getByTestId("timeline-hit").textContent).toContain("Harbour title");
    expect(screen.getByTestId("file-hit").textContent).toContain("Selects");
    await waitFor(() => expect(screen.getByTestId("index-pill").textContent).toBe("Visual search not installed · 0.76 GB"));
    fireEvent.click(screen.getByTestId("index-pill"));
    expect(screen.getByTestId("install-offer").textContent).toContain("Download 0.76 GB");
    expect(screen.getByTestId("install-offer").textContent).toContain("a 0.76 GB download");
    expect(screen.getByTestId("install-model").textContent).toBe("SigLIP 2 base (patch 16, 256 px)");
    expect(screen.getByTestId("install-licence").textContent).toBe("Apache-2.0");
  });

  it("shows Indexing n of N and opens the background tasks from the pill", async () => {
    window.__oveMock!.mediaSearch.status = { ...window.__oveMock!.mediaSearch.status, modelInstalled: true, downloadBytes: 0, indexed: 1, total: 3, running: 2 };
    await panel();
    const task = (id: number) => ({
      id,
      kind: "media-index",
      label: `Indexing clip${id}.mov`,
      project: "/lib/Demo.ove",
      priority: "analysis",
      state: "running",
      pauseReason: null,
      progress: 0.4,
      runningAlone: false,
    });
    act(() => window.__oveMock!.setTasks({ tasks: [task(1), task(2)], paused: [], runAnyway: [] }));
    const pill = await screen.findByTestId("index-pill");
    await waitFor(() => expect(pill.textContent).toBe("Indexing 1 of 3"));
    fireEvent.click(pill);
    expect(await screen.findByTestId("tasks-popover")).toBeTruthy();
    expect(pillText({ ...window.__oveMock!.mediaSearch.status, indexed: 3, total: 3 })?.text).toBe("3 indexed");
  });

  it("dropping a Moment on a track places one clip with the result's range", async () => {
    results();
    await panel();
    await search("harbour");
    const hit = await screen.findByTestId("moment");
    const data = new Map<string, string>();
    const dataTransfer = {
      setData: (k: string, v: string) => data.set(k, v),
      getData: (k: string) => data.get(k) ?? "",
      get types() {
        return [...data.keys()];
      },
      effectAllowed: "",
    };
    fireEvent.dragStart(hit, { dataTransfer });
    const lane = screen.getByTestId("lane-Main");
    const before = window.__oveMock!.calls.filter((c) => c.startsWith("editPlan")).length;
    fireEvent.dragOver(lane, { dataTransfer });
    fireEvent.drop(lane, { dataTransfer, clientX: 0 });
    const plans = window.__oveMock!.calls.filter((c) => c.startsWith("editPlan"));
    expect(plans.length).toBe(before + 1);
    expect(plans.at(-1)).toBe("editPlan:overwrite");
    const plan = window.__oveMock!.lastPlan as { track: string; items: Record<string, unknown>[] };
    expect(plan.track).toBe("t_main");
    expect(plan.items).toEqual([
      { type: "video", mediaItemId: first.id, name: first.name, startTime: 0, endTime: 4, trimStart: 12 },
    ]);
    expect(data.has(CLIP_DRAG_TYPE)).toBe(true);
  });

  it("saves a result as a subclip, which then shows under its media", async () => {
    results();
    await panel();
    await search("harbour");
    fireEvent.click(await screen.findByTestId("save-subclip"));
    await waitFor(() => expect(window.__oveMock!.calls).toContain("mediaOrganize:saveSubclip"));
    fireEvent.change(screen.getByTestId("media-search"), { target: { value: "" } });
    const sub = await screen.findByTestId("subclip");
    expect(sub.textContent).toContain("0:12.0–0:16.0");
  });
});
