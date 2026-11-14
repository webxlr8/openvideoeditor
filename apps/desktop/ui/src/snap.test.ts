import { describe, expect, it } from "vitest";
import { snapToFrame } from "./edit";

describe("snapToFrame", () => {
  it("lands exactly on 30 fps frames, never a millisecond-rounded time", () => {
    expect(snapToFrame(1.0667)).toBe(1.066667);
    expect(snapToFrame(32 / 30)).toBe(1.066667);
    expect(snapToFrame(2.5)).toBe(2.5);
    expect(snapToFrame(-1)).toBe(0);
    for (let f = 0; f < 300; f++) expect(Math.round(snapToFrame(f / 30) * 30)).toBe(f);
  });
});
