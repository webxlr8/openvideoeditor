// Scrub audio requests (add-audio-mixing-and-monitoring, design D6): while the play head is dragged, ask the engine
// for a grain of the mix at the new time, at most 60 times a second. The engine plays only the latest one.

import { commands } from "./bridge";

/** Shortest time between two scrub requests (60 per second). */
export const SCRUB_EVERY_MS = 1000 / 60;

let lastAt = -Infinity;
let lastT = NaN;

/** Requests a grain at timeline time `t` (seconds) unless one was requested within the last 1/60 s. Returns whether it
 * sent one. */
export function scrubAudio(t: number, nowMs = performance.now()): boolean {
  // 1 ms of slack, so pointer events that arrive a hair early in a display frame are not skipped.
  if (nowMs - lastAt < SCRUB_EVERY_MS - 1 || t === lastT || !Number.isFinite(t)) return false;
  lastAt = nowMs;
  lastT = t;
  commands.scrubAudio(t).then(
    () => undefined,
    () => undefined,
  );
  return true;
}

/** Forgets the throttle state (a new drag starts). */
export function resetScrub() {
  lastAt = -Infinity;
  lastT = NaN;
}
