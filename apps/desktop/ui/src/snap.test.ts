import { describe, expect, it } from "vitest";
import { setTimelineFps, snapToFrame } from "./edit";

describe("snapToFrame", () => {
  it("lands exactly on 30 fps frames, never a millisecond-rounded time", () => {
    expect(snapToFrame(1.0667)).toBe(1.066667);
    expect(snapToFrame(32 / 30)).toBe(1.066667);
    expect(snapToFrame(2.5)).toBe(2.5);
    expect(snapToFrame(-1)).toBe(0);
    for (let f = 0; f < 300; f++) expect(Math.round(snapToFrame(f / 30) * 30)).toBe(f);
  });
});

describe("snapToFrame on a 25 fps timeline", () => {
  it("uses the open timeline's frame rate", () => {
    setTimelineFps(25);
    try {
      expect(snapToFrame(1.01)).toBe(1);
      expect(snapToFrame(1.03)).toBe(1.04);
      for (let f = 0; f < 100; f++) expect(snapToFrame(f * 0.04 + 0.013)).toBe(Math.round(f * 40) / 1000);
    } finally {
      setTimelineFps(null);
    }
    expect(snapToFrame(1.0667)).toBe(1.066667);
  });
});
