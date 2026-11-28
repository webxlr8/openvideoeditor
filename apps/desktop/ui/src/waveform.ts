// Clip waveforms and the gain line (add-audio-mixing-and-monitoring, design D3/D4). Pure helpers: the source range
// a clip plays, the heard gain at a clip time (static volume × volume keyframes interpolated in dB × fades, the same
// rule as the mixer), the gain-line geometry, and the waveform drawing. Peaks come from the import analysis cache
// through `peakRange`; gain is applied at draw time, so a volume change redraws without a new query.

import type { ItemView_Serialize, PeakRangeView } from "./bindings";
import { commands } from "./bridge";
import { type EditResult, itemDetail, updateItem } from "./edit";

/** Floor of the volume curve, dB: at or below it a keyframe is silent (the mixer uses the same value). */
export const VOLUME_FLOOR_DB = -60;
/** The gain line spans this range: 0 % at the bottom, 200 % (+6 dB) at the top. */
export const GAIN_MAX = 200;

export interface VolumeKey {
  time: number;
  value: number;
}

export interface ClipAudio {
  volume: number;
  reverse: boolean;
  fadeIn: number;
  fadeOut: number;
  volumeKeys: VolumeKey[];
  volumeEasing: string;
}

const DEFAULT_AUDIO: ClipAudio = { volume: 100, reverse: false, fadeIn: 0, fadeOut: 0, volumeKeys: [], volumeEasing: "linear" };

export function clipAudio(item: ItemView_Serialize): ClipAudio {
  const a = item.audio;
  if (!a) return DEFAULT_AUDIO;
  return {
    volume: a.volume ?? 100,
    reverse: a.reverse,
    fadeIn: a.fadeIn ?? 0,
    fadeOut: a.fadeOut ?? 0,
    volumeKeys: a.volumeKeys.filter((k) => k.time !== null && k.value !== null).map((k) => ({ time: k.time ?? 0, value: k.value ?? 0 })),
    volumeEasing: a.volumeEasing,
  };
}

/** Source seconds a clip plays, from its left edge to its right edge (reversed clips run backwards). */
export function sourceRange(item: ItemView_Serialize): { from: number; to: number; reverse: boolean } {
  const len = Math.max(0, (item.end ?? 0) - (item.start ?? 0)) * (item.speed ?? 1);
  const a = item.trimStart ?? 0;
  const reverse = clipAudio(item).reverse;
  return reverse ? { from: a + len, to: a, reverse } : { from: a, to: a + len, reverse };
}

export function percentToDb(p: number): number {
  return p <= 0 ? VOLUME_FLOOR_DB : Math.max(VOLUME_FLOOR_DB, 20 * Math.log10(p / 100));
}

export function dbToGain(db: number): number {
  return db <= VOLUME_FLOOR_DB ? 0 : Math.pow(10, db / 20);
}

/** Easing curves; the same definitions as the renderer's (ove-core retime). */
export function ease(name: string, p: number): number {
  const t = Math.min(1, Math.max(0, p));
  switch (name) {
    case "ease-in":
      return t * t;
    case "ease-out":
      return 1 - (1 - t) * (1 - t);
    case "ease-in-out":
      return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    default:
      return t;
  }
}

/** Keyframe level in dB at clip time `rel`: held before the first and after the last keyframe. */
export function keyDbAt(keys: VolumeKey[], easing: string, rel: number): number | null {
  if (!keys.length) return null;
  const first = keys[0];
  const last = keys[keys.length - 1];
  if (rel <= first.time) return percentToDb(first.value);
  if (rel >= last.time) return percentToDb(last.value);
  let i = 1;
  while (keys[i].time <= rel) i++;
  const a = keys[i - 1];
  const b = keys[i];
  const p = (rel - a.time) / Math.max(1e-6, b.time - a.time);
  const da = percentToDb(a.value);
  return da + (percentToDb(b.value) - da) * ease(easing, p);
}

/** Linear gain from the static volume and the keyframes (no fades): what the gain line shows. */
export function lineGainAt(a: ClipAudio, rel: number): number {
  const k = keyDbAt(a.volumeKeys, a.volumeEasing, rel);
  return (Math.min(GAIN_MAX, Math.max(0, a.volume)) / 100) * (k === null ? 1 : dbToGain(k));
}

/** The heard gain at clip time `rel` of a clip `len` seconds long: line gain × fades. */
export function gainAt(a: ClipAudio, rel: number, len: number): number {
  let g = lineGainAt(a, rel);
  if (a.fadeIn > 0 && rel < a.fadeIn) g *= Math.max(0, rel / a.fadeIn);
  if (a.fadeOut > 0 && len - rel < a.fadeOut) g *= Math.max(0, (len - rel) / a.fadeOut);
  return g;
}

/** Gain-line y (0 at the top) for a linear gain, in a box `h` pixels high: linear in percent, 200 % at the top. */
export function gainToY(gain: number, h: number): number {
  const p = Math.min(GAIN_MAX, Math.max(0, gain * 100));
  return h - (p / GAIN_MAX) * h;
}

/** Percent for a gain-line y, clamped to 0–200 and rounded to 0.1. */
export function yToPercent(y: number, h: number): number {
  const p = ((h - y) / Math.max(1, h)) * GAIN_MAX;
  return Math.round(Math.min(GAIN_MAX, Math.max(0, p)) * 10) / 10;
}

/** A column's value pair for channel `c` of a peak range. */
function pair(r: PeakRangeView, col: number, c: number): [number, number] {
  const i = (col * r.channels + c) * 2;
  return [r.data[i] / 127, r.data[i + 1] / 127];
}

/**
 * Draws a waveform: one column per pixel, the channels joined into one envelope, each column scaled by `gainOf(x)`
 * (the heard gain at that column). `peaks` holds a source range left to right in source order; `reverse` reads it
 * backwards, so a reversed clip shows its source end at the left edge. Returns the number of columns drawn.
 */
export function drawWaveform(
  ctx: CanvasRenderingContext2D,
  peaks: PeakRangeView,
  reverse: boolean,
  width: number,
  height: number,
  gainOf: (x: number) => number,
  color = "rgba(190, 220, 255, 0.75)",
): number {
  ctx.clearRect(0, 0, width, height);
  if (peaks.status !== "ready" || !peaks.bins || !peaks.channels) return 0;
  ctx.fillStyle = color;
  const mid = height / 2;
  let drawn = 0;
  for (let x = 0; x < width; x++) {
    const col = Math.min(peaks.bins - 1, Math.floor(((reverse ? width - 1 - x : x) + 0.5) * (peaks.bins / width)));
    let lo = 0;
    let hi = 0;
    for (let c = 0; c < peaks.channels; c++) {
      const [a, b] = pair(peaks, col, c);
      lo = Math.min(lo, a);
      hi = Math.max(hi, b);
    }
    const g = gainOf(x);
    const top = Math.max(0, mid - hi * g * mid);
    const bottom = Math.min(height, mid - lo * g * mid);
    if (bottom - top >= 0.5) {
      ctx.fillRect(x, top, 1, bottom - top);
      drawn++;
    }
  }
  return drawn;
}

// ------------------------------------------------------------------------------------------------- peak cache

const cache = new Map<string, Promise<PeakRangeView | null>>();
const MAX_CACHED = 400;

/** Cache key: the media, the source range (ms) and the columns. Gain is not part of it. */
export function peakKey(mediaId: string, from: number, to: number, bins: number): string {
  return `${mediaId}|${Math.round(from * 1000)}|${Math.round(to * 1000)}|${bins}`;
}

/** Peaks of a source range (`from` < `to`) in `bins` columns, cached; pending and error answers are not cached. */
export function loadPeaks(mediaId: string, from: number, to: number, bins: number): Promise<PeakRangeView | null> {
  const key = peakKey(mediaId, from, to, bins);
  const hit = cache.get(key);
  if (hit) return hit;
  const p = commands.peakRange(mediaId, from, to, bins).then((r) => {
    if (r.status !== "ok" || r.data.status !== "ready") cache.delete(key);
    return r.status === "ok" ? r.data : null;
  });
  cache.set(key, p);
  if (cache.size > MAX_CACHED) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  return p;
}

/** Forgets cached peaks (after an analysis finishes, so pending clips ask again). */
export function clearPeaks() {
  cache.clear();
}

// ------------------------------------------------------------------------------------------------- edits

/** Writes a clip's volume keyframes as one revision, keeping its other keyframe tracks. Empty `keys` removes them. */
export async function setVolumeKeys(id: string, keys: VolumeKey[], easing: string, summary: string): EditResult {
  const detail = await itemDetail(id);
  const others = ((detail?.keyframeTracks as { property?: string }[] | undefined) ?? []).filter((t) => t.property !== "volume");
  const sorted = [...keys].sort((a, b) => a.time - b.time).map((k) => ({ time: round(k.time, 6), value: round(k.value, 1) }));
  const tracks = sorted.length ? [...others, { property: "volume", keyframes: sorted, ...(easing !== "linear" ? { easing } : {}) }] : others;
  return tracks.length ? updateItem(id, { keyframeTracks: tracks }, [], summary) : updateItem(id, {}, ["keyframeTracks"], summary);
}

function round(v: number, digits: number): number {
  const f = Math.pow(10, digits);
  return Math.round(v * f) / f;
}
