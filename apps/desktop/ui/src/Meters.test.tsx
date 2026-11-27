import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AudioMeter, MeterReadingView } from "./bindings";
import {
  FALL_DB_PER_S,
  HeardReadings,
  METER_FLOOR_DB,
  PEAK_FALL_DB_PER_S,
  idleChannel,
  idleMeters,
  resetClip,
  stepChannel,
  stepMeters,
} from "./meterBallistics";
import { Meters } from "./Meters";

const FRAME = 1000 / 60;

/** Steps one channel for `ms` at 60 fps with input `db` on the first frame only. */
function run(ms: number, first: number | null, from = idleChannel(), t0 = 0) {
  let s = stepChannel(from, first, first, FRAME, t0);
  let t = t0;
  for (; t < t0 + ms; t += FRAME) s = stepChannel(s, null, null, FRAME, t + FRAME);
  return { s, t };
}

function reading(start: number, peak: number, extra: Partial<MeterReadingView> = {}): MeterReadingView {
  return { start, end: start + 256 / 48000, peak: [peak, peak], rms: [peak - 3, peak - 3], truePeak: [peak, peak], momentary: -20, shortTerm: -21, ...extra };
}

describe("meter ballistics", () => {
  it("the bar falls at 24 dB/s", () => {
    const { s } = run(1000, 0);
    expect(s.level).toBeCloseTo(-FALL_DB_PER_S, 0);
    expect(run(5000, 0).s.level).toBe(METER_FLOOR_DB);
  });

  it("the peak marker holds for 1.5 s, then falls at 18 dB/s", () => {
    const held = run(1400, -6).s;
    expect(held.peak).toBe(-6);
    const { s: falling, t } = run(2500, -6);
    // 18 dB/s from the end of the hold, within one frame's worth of fall.
    expect(Math.abs(falling.peak - (-6 - (PEAK_FALL_DB_PER_S * (t + FRAME - 1500)) / 1000))).toBeLessThan(0.4);
    // A new higher reading raises it at once and restarts the hold.
    const raised = stepChannel(falling, -3, -3, FRAME, 2600);
    expect(raised.peak).toBe(-3);
  });

  it("the clip indicator latches at 0 dBFS until reset", () => {
    let m = stepMeters(idleMeters(), [reading(0, 0)], FRAME, 0);
    expect(m.channels.map((c) => c.clip)).toEqual([true, true]);
    for (let t = 0; t < 5000; t += FRAME) m = stepMeters(m, [], FRAME, t);
    expect(m.channels[0].clip).toBe(true);
    m = resetClip(m);
    expect(m.channels[0].clip).toBe(false);
    // Just below 0 dBFS does not light it; a true peak over 0 does.
    expect(stepMeters(idleMeters(), [reading(0, -0.1)], FRAME, 0).channels[0].clip).toBe(false);
    expect(stepMeters(idleMeters(), [reading(0, -0.5, { truePeak: [0.3, 0.3] })], FRAME, 0).channels[1].clip).toBe(true);
  });

  it("simulated loopback: a click shows within one display frame of being heard", () => {
    // Clicks at media 1, 2 and 3 s. The engine sends batches every 33 ms (readings 40 ms behind to 90 ms ahead of
    // the heard position), delivered 3 ms later; the UI takes readings each display frame. Media time t is heard at
    // wall time t + 0.25 s (start-up plus device latency).
    const R = 256 / 48000;
    const all: MeterReadingView[] = [];
    for (let t = 0; t < 4; t += R) {
      const click = [1, 2, 3].some((c) => c >= t && c < t + R);
      all.push(reading(t, click ? -0.5 : -40));
    }
    const offset = 0.25;
    const h = new HeardReadings();
    const shownAt: number[] = [];
    let nextBatch = 0;
    for (let wall = 0; wall < 4.2; wall += FRAME / 1000) {
      while (nextBatch <= wall) {
        const heard = nextBatch - offset;
        const batch: AudioMeter = { heard, playing: true, readings: all.filter((r) => r.end! >= heard - 0.04 && r.start! <= heard + 0.09) };
        h.receive(batch, (nextBatch + 0.003) * 1000);
        nextBatch += 0.033;
      }
      for (const r of h.take(wall * 1000)) if (r.peak[0]! > -1) shownAt.push(wall);
    }
    expect(shownAt.length).toBe(3);
    const errors = shownAt.map((w, i) => Math.abs(w - (i + 1 + offset)) * 1000);
    expect(Math.max(...errors)).toBeLessThanOrEqual(FRAME + 1e-6);
  });

  it("meter CPU: one display frame of ballistics costs well under 1 % at 60 fps", () => {
    const h = new HeardReadings();
    const rs = Array.from({ length: 30 }, (_, i) => reading(i * (256 / 48000), -12));
    h.receive({ heard: 0.05, playing: true, readings: rs }, 0);
    let m = idleMeters();
    const n = 20000;
    const t0 = performance.now();
    for (let i = 0; i < n; i++) m = stepMeters(m, h.take((i % 30) * 2), FRAME, i * FRAME);
    const perFrameMs = (performance.now() - t0) / n;
    const share = (perFrameMs * 60) / 1000;
    console.log(`meter ballistics: ${(perFrameMs * 1000).toFixed(2)} µs per frame, ${(share * 100).toFixed(3)} % of one core at 60 fps`);
    expect(share).toBeLessThan(0.01);
  });
});

describe("meters panel", () => {
  it("shows the clip indicator after a 0 dBFS reading and clears it on click", async () => {
    render(<Meters />);
    expect(screen.getByTestId("meter-clip-0").className).not.toContain("lit");
    await act(async () => {
      window.__oveMock!.meter({ heard: 1.0, playing: true, readings: [reading(0.999, 0), reading(1.0, -0.2)] });
      await new Promise((r) => setTimeout(r, 120));
    });
    await waitFor(() => expect(screen.getByTestId("meter-clip-0").className).toContain("lit"));
    fireEvent.click(screen.getByTestId("meter-clip-0"));
    await waitFor(() => expect(screen.getByTestId("meter-clip-0").className).not.toContain("lit"));
  });
});
