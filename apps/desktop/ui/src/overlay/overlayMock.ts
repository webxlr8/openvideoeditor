// The browser mock of the viewer overlay commands (UI development and tests): a simplified layer geometry with the
// core's conventions (fractions of the frame, bottom first, corners top-left → bottom-left of the source). Clips fill
// the frame, titles are a box 40 % × 15 % of it, captions sit at the bottom. A title's "ink" has a gap down its
// middle, standing in for the space between letters, so a click there goes to the layer below as in the app.

import type { ProjectView_Serialize, ViewerGeometry, ViewerLayer } from "../bindings";
import { type Crop, type Pt, type Quad, type Transform, NO_CROP, NO_TRANSFORM, contains, referenceSize } from "./geometry";

type Fields = Record<string, unknown>;

const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);

function corners(cx: number, cy: number, w: number, h: number, t: Transform, crop: Crop, rw: number, rh: number) {
  const r = (t.rotation * Math.PI) / 180;
  const [s, c] = [Math.sin(r), Math.cos(r)];
  const flip = t.flipX ? -1 : 1;
  // Source (u, v) in 0..1 → frame fractions.
  const at = (u: number, v: number): Pt => {
    const lx = (u - 0.5) * w * flip;
    const ly = (v - 0.5) * h;
    return [(cx + lx * c - ly * s) / rw, (cy + lx * s + ly * c) / rh];
  };
  const quad: Quad = [at(0, 0), at(1, 0), at(1, 1), at(0, 1)];
  const [l, rr, tt, b] = [crop.left / 100, 1 - crop.right / 100, crop.top / 100, 1 - crop.bottom / 100];
  const cropQuad: Quad = [at(l, tt), at(rr, tt), at(rr, b), at(l, b)];
  return { quad, cropQuad };
}

export function mockGeometry(
  project: ProjectView_Serialize,
  fieldsOf: (id: string) => Fields | undefined,
  time: number,
): ViewerGeometry {
  const [rw, rh] = referenceSize(project.aspectRatio);
  const layers: ViewerLayer[] = [];
  const tracks = [...project.tracks].reverse();
  for (const track of tracks) {
    if (!track.visible || track.kind === "audio") continue;
    const items = project.items
      .filter((i) => i.trackId === track.id && !i.hidden && !i.pending && i.kind !== "audio")
      .filter((i) => (i.start ?? 0) <= time && time < (i.end ?? 0))
      .sort((a, b) => (a.start ?? 0) - (b.start ?? 0));
    for (const item of items) {
      const f = fieldsOf(item.id) ?? {};
      const tf = (f.transform as Fields | undefined) ?? {};
      const t: Transform = {
        x: num(tf.x, NO_TRANSFORM.x),
        y: num(tf.y, NO_TRANSFORM.y),
        scale: num(tf.scale, 1),
        rotation: num(tf.rotation, 0),
        flipX: tf.flipX === true,
      };
      const cf = (f.crop as Fields | undefined) ?? {};
      const media = item.kind === "video" || item.kind === "image";
      const crop: Crop = media
        ? { top: num(cf.top, 0), right: num(cf.right, 0), bottom: num(cf.bottom, 0), left: num(cf.left, 0) }
        : NO_CROP;
      let box: { cx: number; cy: number; w: number; h: number };
      let selectable = true;
      if (media) box = { cx: rw / 2, cy: rh / 2, w: rw, h: rh };
      else if (item.kind === "text") box = { cx: rw / 2, cy: rh / 2, w: rw * 0.4, h: rh * 0.15 };
      else if (item.kind === "caption") box = { cx: rw / 2, cy: rh * 0.85, w: rw * 0.6, h: rh * 0.1 };
      else {
        box = { cx: rw / 2, cy: rh / 2, w: rw, h: rh };
        selectable = false;
      }
      const { quad, cropQuad } = corners(
        box.cx + t.x,
        box.cy + t.y,
        box.w * (selectable ? t.scale : 1),
        box.h * (selectable ? t.scale : 1),
        selectable ? t : NO_TRANSFORM,
        crop,
        rw,
        rh,
      );
      layers.push({
        itemId: item.id,
        itemType: item.kind,
        index: layers.length,
        quad,
        cropQuad,
        rotation: selectable ? t.rotation : 0,
        selectable,
      });
    }
  }
  return { width: rw, height: rh, layers };
}

/** Where (0..1 across, 0..1 down) a point lies in a quad's own axes. */
function local(q: Quad, p: Pt): Pt {
  const ax: Pt = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
  const ay: Pt = [q[3][0] - q[0][0], q[3][1] - q[0][1]];
  const d: Pt = [p[0] - q[0][0], p[1] - q[0][1]];
  const det = ax[0] * ay[1] - ax[1] * ay[0] || 1e-9;
  return [(d[0] * ay[1] - d[1] * ay[0]) / det, (ax[0] * d[1] - ax[1] * d[0]) / det];
}

/** The topmost selectable layer drawn under (x, y) (fractions of the frame), or null. */
export function mockPick(g: ViewerGeometry, x: number, y: number): string | null {
  const p: Pt = [x, y];
  for (const l of [...g.layers].reverse()) {
    if (!l.selectable) continue;
    const cq = l.cropQuad as Quad;
    if (!contains(cq, p)) continue;
    if (l.itemType === "text") {
      const [u] = local(l.quad as Quad, p);
      if (u > 0.45 && u < 0.55) continue;
    }
    return l.itemId;
  }
  return null;
}
