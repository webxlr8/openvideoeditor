import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import type { ItemView_Serialize, PeakRangeView } from "./bindings";
import { clearPeaks, drawWaveform, gainAt, gainToY, keyDbAt, percentToDb, sourceRange, yToPercent, type ClipAudio } from "./waveform";

/** A canvas context that records the columns drawn. */
function recorder() {
  const cols = new Map<number, { top: number; h: number }>();
  const ctx = {
    fillStyle: "",
    clearRect: () => cols.clear(),
    fillRect: (x: number, y: number, _w: number, h: number) => cols.set(x, { top: y, h }),
  } as unknown as CanvasRenderingContext2D;
  return { ctx, cols };
}

/** Peaks whose level rises with source time: column i of n has amplitude (i + 1) / n. */
function ramp(n: number): PeakRangeView {
  const data: number[] = [];
  for (let i = 0; i < n; i++) {
    const v = Math.round(((i + 1) / n) * 127);
    data.push(-v, v);
  }
  return { status: "ready", message: null, contentId: "c", channels: 1, bins: n, level: 0, data };
}

const flat: ClipAudio = { volume: 100, reverse: false, fadeIn: 0, fadeOut: 0, volumeKeys: [], volumeEasing: "linear" };

function item(over: Partial<ItemView_Serialize>): ItemView_Serialize {
  return {
    id: "a",
    trackId: "t",
    kind: "audio",
    name: "a",
    start: 0,
    end: 10,
    trimStart: 10,
    mediaId: "m",
    pending: false,
    hidden: false,
    muted: false,
    text: null,
    transitionIn: 0,
    transitionOut: 0,
    speed: 1,
    link: null,
    ...over,
  };
}

describe("waveform drawing", () => {
  it("maps a trimmed reversed clip: source 20 s at the left edge, 10 s at the right", () => {
    const fwd = sourceRange(item({}));
    expect(fwd).toEqual({ from: 10, to: 20, reverse: false });
    const rev = sourceRange(item({ audio: { volume: 100, reverse: true, fadeIn: 0, fadeOut: 0, volumeKeys: [], volumeEasing: "linear" } }));
    expect(rev).toEqual({ from: 20, to: 10, reverse: true });
    // Speed 2 plays twice the source.
    expect(sourceRange(item({ speed: 2 })).to).toBe(30);
    // Drawn reversed, the loudest (latest) source column lands at the left edge.
    const { ctx, cols } = recorder();
    drawWaveform(ctx, ramp(100), true, 100, 40, () => 1);
    expect(cols.get(0)!.h).toBeGreaterThan(cols.get(90)!.h);
    expect(cols.get(0)!.h).toBeCloseTo(40, 0);
    drawWaveform(ctx, ramp(100), false, 100, 40, () => 1);
    expect(cols.get(99)!.h).toBeGreaterThan(cols.get(9)!.h);
  });

  it("a gain change from 100 to 50 % halves the drawn height without new peaks", () => {
    const peaks = ramp(50);
    const a = recorder();
    drawWaveform(a.ctx, peaks, false, 50, 40, (x) => gainAt(flat, x / 5, 10));
    const b = recorder();
    drawWaveform(b.ctx, peaks, false, 50, 40, (x) => gainAt({ ...flat, volume: 50 }, x / 5, 10));
    for (const x of [10, 25, 49]) expect(b.cols.get(x)!.h).toBeCloseTo(a.cols.get(x)!.h / 2, 0);
  });

  it("the gain curve matches the mixer: a 100 → 0 % fade is -30 dB half way", () => {
    const keys = [
      { time: 0, value: 100 },
      { time: 1, value: 0 },
    ];
    expect(keyDbAt(keys, "linear", 0.5)).toBeCloseTo(-30, 6);
    expect(gainAt({ ...flat, volumeKeys: keys }, 0.5, 10)).toBeCloseTo(Math.pow(10, -30 / 20), 6);
    expect(gainAt({ ...flat, volumeKeys: keys }, 2, 10)).toBe(0);
    expect(percentToDb(200)).toBeCloseTo(6.0206, 3);
    // Fades multiply in.
    expect(gainAt({ ...flat, fadeIn: 2 }, 1, 10)).toBeCloseTo(0.5, 6);
    // The gain line maps 0–200 % linearly and clamps.
    expect(gainToY(1, 40)).toBe(20);
    expect(yToPercent(-50, 40)).toBe(200);
    expect(yToPercent(90, 40)).toBe(0);
  });

  it("draws 50 visible clips quickly", () => {
    const peaks = ramp(1200);
    const { ctx } = recorder();
    const audio = { ...flat, volumeKeys: [{ time: 0, value: 100 }, { time: 5, value: 20 }, { time: 10, value: 100 }] };
    const t0 = performance.now();
    for (let c = 0; c < 50; c++) drawWaveform(ctx, peaks, c % 2 === 1, 1200, 33, (x) => gainAt(audio, x / 120, 10));
    const ms = performance.now() - t0;
    console.log(`waveform draw: 50 clips x 1200 px in ${ms.toFixed(1)} ms`);
    expect(ms).toBeLessThan(500);
  });
});

async function editorWithClip() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  return screen.findByTestId("gain-line");
}


async function gesture(target: Element, from: { x?: number; y?: number; alt?: boolean }, to?: { x?: number; y?: number }) {
  fireEvent.pointerDown(target, { button: 0, clientX: from.x ?? 0, clientY: from.y ?? 0, altKey: from.alt ?? false });
  await act(async () => {
    if (to) window.dispatchEvent(new MouseEvent("pointermove", { clientX: to.x ?? from.x ?? 0, clientY: to.y ?? from.y ?? 0 }));
    window.dispatchEvent(new MouseEvent("pointerup", { clientX: to?.x ?? 0, clientY: to?.y ?? 0 }));
  });
}

describe("clip waveform and gain line", () => {
  it("shows the waveform from the peak cache", async () => {
    await editorWithClip();
    const wave = await screen.findByTestId("waveform");
    expect(Number(wave.getAttribute("data-cols"))).toBeGreaterThan(0);
    expect(screen.getByTestId("clip-audio").getAttribute("data-wave")).toBe("ready");
  });

  it("dragging the line down to 50 % sets the volume in one revision, and undo restores it", async () => {
    const line = await editorWithClip();
    expect(line.getAttribute("data-volume")).toBe("100");
    const edits = vi.spyOn(commands, "updateItem");
    // Clip height 33 px spans 0–200 %: 50 % is 8.25 px below 100 %.
    const hit = screen.getByTestId("gain-hit");
    fireEvent.pointerDown(hit, { button: 0, clientY: 100 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientY: 104 }));
      window.dispatchEvent(new MouseEvent("pointermove", { clientY: 108.25 }));
    });
    expect(screen.getByTestId("gain-readout").textContent).toBe("50 %");
    expect(edits.mock.calls.length).toBe(0); // Nothing is committed while the pointer is down.
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientY: 108.25 }));
    });
    await waitFor(() => expect(screen.getByTestId("gain-line").getAttribute("data-volume")).toBe("50"));
    expect(edits.mock.calls.length).toBe(1); // One revision for the gesture.
    edits.mockRestore();
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(screen.getByTestId("gain-line").getAttribute("data-volume")).toBe("100"));
  });

  it("dragging above the limit stops at 200 %", async () => {
    await editorWithClip();
    const hit = screen.getByTestId("gain-hit");
    fireEvent.pointerDown(hit, { button: 0, clientY: 100 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientY: 20 }));
    });
    expect(screen.getByTestId("gain-readout").textContent).toBe("200 %");
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientY: 20 }));
    });
    await waitFor(() => expect(screen.getByTestId("gain-line").getAttribute("data-volume")).toBe("200"));
  });

  it("Alt-click adds a keyframe, a point drag moves it, Delete removes it; each is one undo step", async () => {
    await editorWithClip();
    await gesture(screen.getByTestId("gain-hit"), { x: 0, y: 16, alt: true });
    await waitFor(() => expect(screen.getAllByTestId("gain-point").length).toBe(1));
    expect(screen.getByTestId("gain-line").getAttribute("data-keys")).toBe("1");
    // Drag the point down: its level falls; one revision on release.
    const point = screen.getByTestId("gain-point");
    const y0 = Number(point.getAttribute("cy"));
    await gesture(point, { x: 0, y: 100 }, { x: 30, y: 108.25 });
    await waitFor(() => expect(Number(screen.getByTestId("gain-point").getAttribute("cy"))).toBeGreaterThan(y0 + 7));
    expect(Number(screen.getByTestId("gain-point").getAttribute("cx"))).toBeGreaterThan(0);
    // Undo the move: the point returns.
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(Number(screen.getByTestId("gain-point").getAttribute("cy"))).toBeCloseTo(y0, 1));
    // Select and delete it.
    await gesture(screen.getByTestId("gain-point"), { x: 0, y: 0 });
    fireEvent.keyDown(screen.getByTestId("gain-line"), { key: "Delete" });
    await waitFor(() => expect(screen.queryAllByTestId("gain-point").length).toBe(0));
    expect(screen.getAllByTestId("clip").length).toBe(1); // The clip itself is not deleted.
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(screen.getAllByTestId("gain-point").length).toBe(1));
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(screen.queryAllByTestId("gain-point").length).toBe(0));
  });
});

describe("clips without analysis", () => {
  const answer = (status: string, message: string | null = null) => ({
    status: "ok" as const,
    data: { status, message, contentId: "c", channels: 0, bins: 0, level: 0, data: [] },
  });

  it("show a placeholder while the analysis is pending", async () => {
    clearPeaks();
    const spy = vi.spyOn(commands, "peakRange").mockResolvedValue(answer("pending"));
    await editorWithClip();
    await screen.findByTestId("wave-placeholder");
    expect(screen.queryByTestId("waveform")).toBeNull();
    spy.mockRestore();
    clearPeaks();
  });

  it("show no waveform, only a marker, when the analysis failed", async () => {
    clearPeaks();
    const spy = vi.spyOn(commands, "peakRange").mockResolvedValue(answer("error", "The sound could not be analysed"));
    await editorWithClip();
    const marker = await screen.findByTestId("wave-error");
    expect(marker.getAttribute("title")).toContain("could not be analysed");
    expect(screen.queryByTestId("waveform")).toBeNull();
    expect(screen.queryByTestId("wave-placeholder")).toBeNull();
    spy.mockRestore();
    clearPeaks();
  });
});
