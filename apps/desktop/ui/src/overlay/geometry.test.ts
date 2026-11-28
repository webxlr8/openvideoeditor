import { describe, expect, it } from "vitest";
import {
  type Quad,
  NO_CROP,
  NO_TRANSFORM,
  contains,
  cropQuad,
  cropped,
  moved,
  rotated,
  scaled,
  snapMove,
  turnScale,
} from "./geometry";

const box: Quad = [
  [100, 50],
  [200, 50],
  [200, 100],
  [100, 100],
];

describe("overlay geometry", () => {
  it("snaps the centre and, unrotated, the edges within 8 px", () => {
    // Centre 150 → 158 (2 px from 160): snapped onto the canvas centre.
    expect(snapMove(box, [8, 0], 320, 180, 0).delta).toEqual([10, 0]);
    // Left edge 100 → 5: snapped to 0.
    const edge = snapMove(box, [-95, 0], 320, 180, 0);
    expect(edge.delta).toEqual([-100, 0]);
    expect(edge.lines.x).toEqual([0]);
    // Rotated: edges do not snap, the centre still does.
    expect(snapMove(box, [-95, 0], 320, 180, 30).delta).toEqual([-95, 0]);
    expect(snapMove(box, [8, 0], 320, 180, 30).delta).toEqual([10, 0]);
    // Far from everything: unchanged.
    expect(snapMove(box, [30, 30], 320, 180, 0)).toEqual({ delta: [30, 30], lines: { x: [], y: [] } });
  });

  it("converts screen pixels to reference pixels", () => {
    expect(moved(NO_TRANSFORM, [32, -16], 320, 1920)).toMatchObject({ x: 192, y: -96 });
  });

  it("scales uniformly around the centre", () => {
    expect(scaled(NO_TRANSFORM, [0, 0], [10, 0], [0, 25]).scale).toBe(2.5);
  });

  it("rotates around the centre, wraps to ±180 and steps by 15° with Shift", () => {
    expect(rotated(NO_TRANSFORM, [0, 0], [0, -10], [10, 0], false).rotation).toBe(90);
    expect(rotated({ ...NO_TRANSFORM, rotation: 170 }, [0, 0], [0, -10], [10, 0], false).rotation).toBe(-100);
    expect(rotated(NO_TRANSFORM, [0, 0], [0, -10], [10, -9], true).rotation).toBe(45);
  });

  it("crops along the layer's own axes and keeps 1 %", () => {
    // Turned a quarter: the source's left→right runs down the screen.
    const turned = turnScale(box, [150, 75], 90, 1);
    const c = cropped(NO_CROP, turned, "right", [0, -25]);
    expect(c.right).toBeCloseTo(25, 5);
    expect(cropped(NO_CROP, box, "left", [500, 0]).left).toBe(99);
    expect(cropped({ ...NO_CROP, right: 40 }, box, "left", [500, 0]).left).toBe(59);
    const q = cropQuad(box, { ...NO_CROP, right: 50 });
    expect(q[1]).toEqual([150, 50]);
    expect(contains(q, [140, 60])).toBe(true);
    expect(contains(q, [160, 60])).toBe(false);
  });
});
