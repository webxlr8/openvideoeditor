// Geometry for the viewer overlay (add-viewer-interaction-and-text-fills, design D2/D3): layer corners from the core
// (fractions of the frame) turned into viewer pixels, and the transform, scale, rotation and crop a handle drag
// gives, with snapping. Pure functions, so the gestures are unit-tested without a viewer.

export type Pt = [number, number];
export type Quad = [Pt, Pt, Pt, Pt];

/** The item fields a handle changes, as the document stores them. */
export type Transform = { x: number; y: number; scale: number; rotation: number; flipX?: boolean };
export type Crop = { top: number; right: number; bottom: number; left: number };

export const NO_TRANSFORM: Transform = { x: 0, y: 0, scale: 1, rotation: 0 };
export const NO_CROP: Crop = { top: 0, right: 0, bottom: 0, left: 0 };

/** Move snapping distance, in screen pixels (design D3). */
export const SNAP_PX = 8;
/** Rotation step while Shift is held, in degrees. */
export const ROTATE_STEP = 15;

/** Reference frame size of an aspect ratio: the pixel space of transforms (as the core's `reference_size`). */
export function referenceSize(aspect: string): [number, number] {
  switch (aspect) {
    case "9:16":
      return [1080, 1920];
    case "1:1":
      return [1080, 1080];
    case "4:5":
      return [1080, 1350];
    case "4:3":
      return [1440, 1080];
    case "3:4":
      return [1080, 1440];
    default:
      return [1920, 1080];
  }
}

/** Corners as fractions of the frame → pixels of a `w`×`h` box. */
export function toBox(q: (number | null)[][], w: number, h: number): Quad {
  const p = (i: number): Pt => [(q[i]?.[0] ?? 0) * w, (q[i]?.[1] ?? 0) * h];
  return [p(0), p(1), p(2), p(3)];
}

export const add = (a: Pt, b: Pt): Pt => [a[0] + b[0], a[1] + b[1]];
export const sub = (a: Pt, b: Pt): Pt => [a[0] - b[0], a[1] - b[1]];
export const mid = (a: Pt, b: Pt): Pt => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
export const len = (a: Pt): number => Math.hypot(a[0], a[1]);
const dot = (a: Pt, b: Pt) => a[0] * b[0] + a[1] * b[1];

export function centre(q: Quad): Pt {
  return mid(q[0], q[2]);
}

export function bounds(q: Quad): { x0: number; y0: number; x1: number; y1: number } {
  const xs = q.map((p) => p[0]);
  const ys = q.map((p) => p[1]);
  return { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) };
}

/** True when `p` is inside the convex quad (either winding; edges count). */
export function contains(q: Quad, p: Pt): boolean {
  let sign = 0;
  for (let i = 0; i < 4; i++) {
    const a = q[i];
    const b = q[(i + 1) % 4];
    const c = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    if (Math.abs(c) < 1e-9) continue;
    if (sign === 0) sign = Math.sign(c);
    else if (Math.sign(c) !== sign) return false;
  }
  return true;
}

export function translate(q: Quad, d: Pt): Quad {
  return [add(q[0], d), add(q[1], d), add(q[2], d), add(q[3], d)];
}

/** The quad turned by `deg` (clockwise) and scaled by `k` around `c`. */
export function turnScale(q: Quad, c: Pt, deg: number, k: number): Quad {
  const r = (deg * Math.PI) / 180;
  const [s, co] = [Math.sin(r), Math.cos(r)];
  const f = (p: Pt): Pt => {
    const [x, y] = sub(p, c);
    return [c[0] + k * (x * co - y * s), c[1] + k * (x * s + y * co)];
  };
  return [f(q[0]), f(q[1]), f(q[2]), f(q[3])];
}

/** True when a rotation is a whole number of quarter turns away from upright, so the box edges are horizontal. */
function upright(rotation: number): boolean {
  const r = ((rotation % 360) + 360) % 360;
  return r < 1e-6 || 360 - r < 1e-6;
}

/**
 * Snaps a move of `delta` (screen pixels) of a layer whose box is `q` in a `w`×`h` viewer: its centre to the canvas
 * centre lines, and — when the layer is not rotated — its edges to the canvas edges, each within `SNAP_PX`. Returns
 * the snapped delta and the guide lines it snapped to (x and y in screen pixels) for drawing.
 */
export function snapMove(
  q: Quad,
  delta: Pt,
  w: number,
  h: number,
  rotation: number,
): { delta: Pt; lines: { x: number[]; y: number[] } } {
  const moved = translate(q, delta);
  const b = bounds(moved);
  const c = centre(moved);
  const xs: [number, number][] = [[c[0], w / 2]];
  const ys: [number, number][] = [[c[1], h / 2]];
  if (upright(rotation)) {
    xs.push([b.x0, 0], [b.x1, w]);
    ys.push([b.y0, 0], [b.y1, h]);
  }
  const best = (cands: [number, number][]) => {
    let pick: [number, number] | null = null;
    for (const [at, target] of cands) {
      const d = target - at;
      if (Math.abs(d) <= SNAP_PX && (!pick || Math.abs(d) < Math.abs(pick[0]))) pick = [d, target];
    }
    return pick;
  };
  const sx = best(xs);
  const sy = best(ys);
  return {
    delta: [delta[0] + (sx?.[0] ?? 0), delta[1] + (sy?.[0] ?? 0)],
    lines: { x: sx ? [sx[1]] : [], y: sy ? [sy[1]] : [] },
  };
}

/** The transform after moving by `delta` screen pixels in a viewer `boxW` pixels wide showing `refW` reference pixels. */
export function moved(t: Transform, delta: Pt, boxW: number, refW: number): Transform {
  const k = refW / Math.max(boxW, 1);
  return { ...t, x: Math.round(t.x + delta[0] * k), y: Math.round(t.y + delta[1] * k) };
}

/** The transform after dragging a corner from `from` to `to` around the layer's centre `c` (uniform scale). */
export function scaled(t: Transform, c: Pt, from: Pt, to: Pt): Transform {
  const k = len(sub(to, c)) / Math.max(len(sub(from, c)), 1e-6);
  const scale = Math.min(20, Math.max(0.01, t.scale * k));
  return { ...t, scale: Math.round(scale * 1000) / 1000 };
}

/** The transform after dragging the rotation handle from `from` to `to` around `c`; Shift snaps to 15° steps. */
export function rotated(t: Transform, c: Pt, from: Pt, to: Pt, shift: boolean): Transform {
  const a = (p: Pt) => (Math.atan2(p[1] - c[1], p[0] - c[0]) * 180) / Math.PI;
  let rotation = t.rotation + a(to) - a(from);
  rotation = ((((rotation + 180) % 360) + 360) % 360) - 180;
  rotation = shift ? Math.round(rotation / ROTATE_STEP) * ROTATE_STEP : Math.round(rotation * 10) / 10;
  if (rotation === -180) rotation = 180;
  return { ...t, rotation };
}

export type Side = "top" | "right" | "bottom" | "left";

/**
 * The crop after dragging the `side` crop handle by `delta` screen pixels. `q` is the whole source's quad (its
 * top-left, top-right, bottom-right, bottom-left on screen), so the drag is measured along the layer's own axes: a
 * rotated or mirrored layer crops along its own edge. Each side keeps at least 1 % of the source.
 */
export function cropped(c: Crop, q: Quad, side: Side, delta: Pt): Crop {
  const across = sub(q[1], q[0]); // source left → right
  const down = sub(q[3], q[0]); // source top → bottom
  const along = (axis: Pt) => (dot(delta, axis) / Math.max(dot(axis, axis), 1e-9)) * 100;
  const next = { ...c };
  const clamp = (v: number, other: number) => Math.round(Math.min(99 - other, Math.max(0, v)) * 10) / 10;
  if (side === "left") next.left = clamp(c.left + along(across), c.right);
  if (side === "right") next.right = clamp(c.right - along(across), c.left);
  if (side === "top") next.top = clamp(c.top + along(down), c.bottom);
  if (side === "bottom") next.bottom = clamp(c.bottom - along(down), c.top);
  return next;
}

/** Where the crop handles sit: the middle of each edge of the cropped quad. */
export function cropHandles(cq: Quad): Record<Side, Pt> {
  return { top: mid(cq[0], cq[1]), right: mid(cq[1], cq[2]), bottom: mid(cq[2], cq[3]), left: mid(cq[3], cq[0]) };
}

/** Where the rotation handle sits: `gap` pixels outside the middle of the top edge. */
export function rotateHandle(cq: Quad, gap = 28): Pt {
  const top = mid(cq[0], cq[1]);
  const c = centre(cq);
  const out = sub(top, c);
  const l = len(out);
  return l < 1e-6 ? [top[0], top[1] - gap] : add(top, [(out[0] / l) * gap, (out[1] / l) * gap]);
}

/** A crop quad from a whole-source quad and a crop (for the live outline while dragging). */
export function cropQuad(q: Quad, c: Crop): Quad {
  const across = sub(q[1], q[0]);
  const down = sub(q[3], q[0]);
  const at = (u: number, v: number): Pt => [
    q[0][0] + across[0] * u + down[0] * v,
    q[0][1] + across[1] * u + down[1] * v,
  ];
  const [l, r, t, b] = [c.left / 100, 1 - c.right / 100, c.top / 100, 1 - c.bottom / 100];
  return [at(l, t), at(r, t), at(r, b), at(l, b)];
}
