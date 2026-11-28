// The key colour sampler's state (add-gpu-compositor-and-scopes 5.2): a chroma key's "Sample" button arms it, and the
// viewer then takes one click (a point) or drag (a rectangle) on the frame and sets the key colour to the source
// colour under it, before grading, as one revision.

import { useSyncExternalStore } from "react";

export type KeySampling = { itemId: string; effectId: string; time: number };

let current: KeySampling | null = null;
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

/** Arms the sampler for chroma key `effectId` on `itemId` at timeline time `time` (seconds). */
export function startKeySampling(s: KeySampling) {
  current = s;
  emit();
}

export function stopKeySampling() {
  current = null;
  emit();
}

/** The armed sampler, or null. */
export function useKeySampling(): KeySampling | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
  );
}

/** A drag from (x0, y0) to (x1, y1) in pixels inside a box of `w`×`h` pixels as frame fractions, clamped. */
export function sampleRect(x0: number, y0: number, x1: number, y1: number, w: number, h: number): number[] {
  const fx = (x: number) => Math.min(1, Math.max(0, x / Math.max(w, 1)));
  const fy = (y: number) => Math.min(1, Math.max(0, y / Math.max(h, 1)));
  // Under 3 pixels of movement is a click: a point.
  if (Math.abs(x1 - x0) < 3 && Math.abs(y1 - y0) < 3) return [fx(x0), fy(y0), fx(x0), fy(y0)];
  return [fx(Math.min(x0, x1)), fy(Math.min(y0, y1)), fx(Math.max(x0, x1)), fy(Math.max(y0, y1))];
}
