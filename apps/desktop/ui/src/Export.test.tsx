import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
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

  it("steps one frame per 40 ms on a 25 fps timeline", async () => {
    window.__oveMock!.fps = 25;
    try {
      await openEditor();
      await waitFor(() => expect(window.__oveMock!.calls).toContain("openTimeline"));
      for (let i = 0; i < 25; i++) fireEvent.keyDown(window, { key: "ArrowRight" });
      // 25 steps of 40 ms make one second.
      await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:01:00"));
    } finally {
      window.__oveMock!.fps = undefined;
    }
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
  it("writes a timeline file straight away when nothing is lost", async () => {
    await openEditor();
    fireEvent.click(screen.getByTestId("open-export"));
    fireEvent.click(await screen.findByTestId("export-kind-timeline"));
    fireEvent.click(screen.getByTestId("timeline-export-start"));
    const done = await screen.findByTestId("timeline-export-done");
    expect(done.textContent).toContain(".fcpxml");
    expect(window.__oveMock!.calls).toEqual(
      expect.arrayContaining(["exportTimelineReport:fcpxml@30", "exportTimeline:fcpxml@30"]),
    );
  });

  it("shows what won't carry over before writing, and continues on request", async () => {
    await openEditor();
    // A title and a caption: an EDL has neither.
    fireEvent.click(screen.getByTestId("rail-text"));
    fireEvent.click(await screen.findByTestId("text-preset-headline"));
    fireEvent.click(screen.getByTestId("rail-captions"));
    fireEvent.click(await screen.findByTestId("caption-add"));
    await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(3));

    fireEvent.click(screen.getByTestId("open-export"));
    fireEvent.click(await screen.findByTestId("export-kind-timeline"));
    fireEvent.click(within(screen.getByTestId("timeline-formats")).getByText("EDL"));
    fireEvent.click(screen.getByTestId("timeline-export-start"));
    const report = await screen.findByTestId("timeline-report");
    expect(report.textContent).toContain("What won't carry over");
    const lines = screen.getAllByTestId("timeline-omitted").map((l) => l.textContent);
    expect(lines).toEqual(
      expect.arrayContaining([
        expect.stringContaining("Captions (1) — timeline files have no caption track"),
        expect.stringContaining("Titles (1) — an EDL has no titles"),
      ]),
    );
    // Nothing written yet.
    expect(window.__oveMock!.calls).not.toContain("exportTimeline:edl@30");
    expect(screen.queryByTestId("timeline-export-done")).toBeNull();

    fireEvent.click(screen.getByTestId("timeline-export-continue"));
    expect((await screen.findByTestId("timeline-export-done")).textContent).toContain(".edl");
    expect(window.__oveMock!.calls).toContain("exportTimeline:edl@30");
  });

});

const calls = () => window.__oveMock!.calls;

async function openDialog() {
  fireEvent.click(screen.getByTestId("open-export"));
  const dialog = await screen.findByTestId("export-dialog");
  await within(dialog).findByTestId("export-presets");
  return dialog;
}

/** Waits for the dialog's check of the current settings. */
const settled = () =>
  waitFor(() =>
    expect(
      screen.queryByTestId("export-summary")?.textContent || screen.queryByTestId("export-invalid")?.textContent,
    ).toBeTruthy(),
  );

describe("export queue: dialog", () => {
  afterEach(() => {
    window.__oveMock!.exportHold = false;
  });

  it("queues an export with a preset, shows its progress and the result", async () => {
    await openEditor();
    const dialog = await openDialog();
    fireEvent.click(within(dialog).getByTestId("export-preset-prores-422-hq-master"));
    expect(within(dialog).getByTestId("export-preset-prores-422-hq-master").getAttribute("aria-checked")).toBe("true");
    await waitFor(() => expect(screen.getByTestId("export-summary").textContent).toContain("ProRes 422 HQ"));
    expect(screen.getByTestId("export-summary").textContent).toContain("1920×1080");
    // A preset this computer cannot write is shown, not offered.
    expect(within(dialog).getByTestId<HTMLButtonElement>("export-preset-hevc-10-bit").disabled).toBe(true);
    fireEvent.click(screen.getByTestId("export-start"));
    await screen.findByTestId("export-done", undefined, { timeout: 3000 });
    expect(calls()).toEqual(expect.arrayContaining(["chooseExportPath:mov", "queueExport:mov:prores"]));
    expect(screen.getByTestId("export-done").textContent).toContain(".mov");
    fireEvent.click(screen.getByTestId("export-reveal"));
    expect(calls().some((c) => c.startsWith("revealPath:"))).toBe(true);
  });

  it("refuses an impossible combination and names the conflicting setting", async () => {
    await openEditor();
    const queued = calls().filter((c) => c.startsWith("queueExport")).length;
    const dialog = await openDialog();
    fireEvent.click(within(dialog).getByTestId("export-preset-youtube-1080p"));
    await settled();
    fireEvent.click(within(dialog).getByTestId("export-alpha"));
    const why = await screen.findByTestId("export-invalid");
    expect(why.textContent).toContain("transparency");
    expect(screen.getByTestId<HTMLButtonElement>("export-start").disabled).toBe(true);
    expect(screen.getByTestId("export-alpha").closest(".xp-invalid")).not.toBeNull();
    expect(screen.getByTestId("export-video-codec").closest(".xp-invalid")).not.toBeNull();
    // A container that cannot hold the codec gets one it can, instead of a refusal.
    fireEvent.click(within(dialog).getByTestId("export-alpha"));
    fireEvent.change(screen.getByTestId("export-container"), { target: { value: "mov" } });
    fireEvent.change(screen.getByTestId("export-video-codec"), { target: { value: "prores" } });
    await waitFor(() => expect(screen.getByTestId("export-summary").textContent).toContain("MOV · ProRes"));
    fireEvent.change(screen.getByTestId("export-container"), { target: { value: "mp4" } });
    await waitFor(() => expect(screen.getByTestId("export-summary").textContent).toContain("MP4 · H.264"));
    expect(calls().filter((c) => c.startsWith("queueExport")).length).toBe(queued);
  });

  it("exports the mix alone as WAV with a loudness target", async () => {
    await openEditor();
    await openDialog();
    fireEvent.click(screen.getByTestId("export-audio-only"));
    fireEvent.change(screen.getByTestId("export-loudness"), { target: { value: "-14" } });
    await waitFor(() => expect(screen.getByTestId("export-summary").textContent).toBe("WAV · PCM 24-bit · -14 LUFS"));
    fireEvent.click(screen.getByTestId("export-start"));
    const done = await screen.findByTestId("export-done", undefined, { timeout: 3000 });
    expect(done.textContent).toContain(".wav");
    expect(done.textContent).toContain("-14.0 LUFS");
    expect(calls()).toContain("queueExport:wav:audio:-14LUFS");
  });

  it("refuses a range outside the timeline and exports a range inside it", async () => {
    await openEditor();
    await openDialog();
    fireEvent.click(screen.getByTestId("export-range-part"));
    fireEvent.change(screen.getByTestId("export-range-start"), { target: { value: "2" } });
    fireEvent.change(screen.getByTestId("export-range-end"), { target: { value: "5000" } });
    expect((await screen.findByTestId("export-invalid")).textContent).toContain("outside the timeline");
    fireEvent.change(screen.getByTestId("export-range-end"), { target: { value: "4" } });
    await waitFor(() => expect(screen.getByTestId("export-summary")).toBeTruthy());
    fireEvent.click(screen.getByTestId("export-start"));
    await screen.findByTestId("export-done", undefined, { timeout: 3000 });
    expect(calls().some((c) => c.startsWith("queueExport:") && c.endsWith("@2-4"))).toBe(true);
  });

  it("cancels a queued export from the dialog", async () => {
    window.__oveMock!.exportHold = true;
    await openEditor();
    await openDialog();
    await settled();
    fireEvent.click(screen.getByTestId("export-start"));
    const progress = await screen.findByTestId("export-progress");
    expect(progress.getAttribute("data-status")).toBe("queued");
    fireEvent.click(screen.getByTestId("export-cancel"));
    const err = await screen.findByTestId("export-error");
    expect(err.textContent).toContain("cancelled");
    expect(calls().some((c) => c.startsWith("cancelExportJob:"))).toBe(true);
  });
});

describe("export queue: Exports panel", () => {
  afterEach(() => {
    window.__oveMock!.exportHold = false;
  });

  it("lists queued jobs, cancels one, finishes the other and exports again", async () => {
    window.__oveMock!.exportHold = true;
    await openEditor();
    for (const preset of ["youtube-1080p", "audio-wav-24-bit"]) {
      const dialog = await openDialog();
      fireEvent.click(within(dialog).getByTestId(`export-preset-${preset}`));
      await settled();
      fireEvent.click(screen.getByTestId("export-start"));
      await screen.findByTestId("export-progress");
      fireEvent.click(screen.getByTestId("export-show-exports"));
      await waitFor(() => expect(screen.queryByTestId("export-dialog")).toBeNull());
    }
    const panel = await screen.findByTestId("exports-panel");
    const rows = () => within(panel).getAllByTestId("export-job");
    await waitFor(() => expect(rows().filter((r) => r.getAttribute("data-status") === "queued").length).toBe(2));
    const queued = rows().filter((r) => r.getAttribute("data-status") === "queued");
    const firstId = queued[0].getAttribute("data-id");
    const secondId = queued[1].getAttribute("data-id");
    const byId = (id: string | null) => rows().find((r) => r.getAttribute("data-id") === id)!;
    fireEvent.click(within(byId(firstId)).getByTestId("export-job-cancel"));
    await waitFor(() => expect(byId(firstId).getAttribute("data-status")).toBe("cancelled"));
    window.__oveMock!.exportHold = false;
    await waitFor(() => expect(byId(secondId).getAttribute("data-status")).toBe("done"), { timeout: 5000 });
    expect(byId(firstId).getAttribute("data-status")).toBe("cancelled");
    fireEvent.click(within(byId(firstId)).getByTestId("export-job-again"));
    await waitFor(() => expect(calls()).toContain(`exportAgain:${firstId}`));
  });
});

describe("timeline range menu", () => {
  it("saves a marked range as media, and renders in place as one undo step", async () => {
    await openEditor();
    const before = screen.getAllByTestId("clip").length;
    fireEvent.pointerDown(screen.getAllByTestId("clip")[0], { button: 0, clientX: 400 });
    fireEvent.pointerUp(window);
    fireEvent.keyDown(window, { key: "x" });
    const range = await screen.findByTestId("range");
    const [a, b] = [Number(range.getAttribute("data-in")), Number(range.getAttribute("data-out"))];

    fireEvent.contextMenu(range);
    fireEvent.click(await screen.findByTestId("menu-save-range"));
    expect((await screen.findByTestId("range-done")).textContent).toContain("Added to your media");
    expect(calls()).toContain(`saveRangeAsMedia:${a}-${b}`);
    fireEvent.click(screen.getByText("Done"));
    expect(screen.getAllByTestId("clip").length).toBe(before);

    fireEvent.contextMenu(screen.getByTestId("range"));
    fireEvent.click(await screen.findByTestId("menu-render-in-place"));
    // Asked first; nothing changes until confirmed.
    await screen.findByTestId("range-confirm");
    expect(calls()).not.toContain(`saveRangeAsMedia:${a}-${b}:replace`);
    fireEvent.click(screen.getByTestId("range-confirm"));
    expect((await screen.findByTestId("range-done")).textContent).toContain("Rendered in place");
    fireEvent.click(screen.getByText("Done"));
    const names = () => screen.getAllByTestId("clip").map((c) => c.textContent);
    await waitFor(() => expect(names().some((n) => n?.includes(`Range ${a}–${b} s`))).toBe(true));

    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(names().some((n) => n?.includes("Range"))).toBe(false));
    expect(screen.getAllByTestId("clip").length).toBe(before);
  });

  it("opens the export dialog for the marked range", async () => {
    await openEditor();
    fireEvent.pointerDown(screen.getAllByTestId("clip")[0], { button: 0, clientX: 400 });
    fireEvent.pointerUp(window);
    fireEvent.keyDown(window, { key: "x" });
    fireEvent.contextMenu(await screen.findByTestId("range"));
    fireEvent.click(await screen.findByTestId("menu-export-range"));
    await screen.findByTestId("export-presets");
    expect(screen.getByTestId("export-range-part").getAttribute("aria-checked")).toBe("true");
    await waitFor(() => expect(screen.getByTestId("export-summary").textContent).toMatch(/· [\d.]+–[\d.]+ s$/));
  });
});
