import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import { SCRUB_EVERY_MS, resetScrub, scrubAudio } from "./scrubAudio";

describe("scrub audio", () => {
  it("asks for at most 60 grains a second", () => {
    const spy = vi.spyOn(commands, "scrubAudio");
    resetScrub();
    let sent = 0;
    // 240 pointer moves over one second (4 per display frame).
    for (let i = 0; i < 240; i++) if (scrubAudio(i * 0.01, i * (1000 / 240))) sent++;
    expect(sent).toBeLessThanOrEqual(61);
    expect(sent).toBeGreaterThanOrEqual(59);
    expect(spy).toHaveBeenCalledTimes(sent);
    // The same time twice is not asked again.
    resetScrub();
    expect(scrubAudio(1, 0)).toBe(true);
    expect(scrubAudio(1, SCRUB_EVERY_MS * 2)).toBe(false);
    spy.mockRestore();
  });

  it("dragging along the ruler plays grains at the new times", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
    const before = window.__oveMock!.calls.filter((c) => c.startsWith("scrubAudio")).length;
    fireEvent.pointerDown(screen.getByTestId("ruler"), { button: 0, clientX: 300 });
    for (const x of [320, 340, 360]) {
      await act(async () => {
        window.dispatchEvent(new MouseEvent("pointermove", { clientX: x }));
        await new Promise((r) => setTimeout(r, 20));
      });
    }
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 360 }));
    });
    const calls = window.__oveMock!.calls.filter((c) => c.startsWith("scrubAudio")).slice(before);
    expect(calls.length).toBeGreaterThanOrEqual(3);
    const times = calls.map((c) => Number(c.split(":")[1]));
    expect(times[times.length - 1]).toBeGreaterThan(times[0]);
  });
});
