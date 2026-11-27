// Level-meter logic (add-audio-mixing-and-monitoring, design D5), pure so it is unit-tested:
// - which readings of the engine's batches are under the heard position at a given moment;
// - the ballistics: the bar falls at 24 dB/s, the peak marker holds for 1.5 s and then falls at 18 dB/s, and the
//   clip indicator stays lit from the first reading at or above 0 dBFS until the person clicks it.

import type { AudioMeter, MeterReadingView } from "./bindings";

export const METER_FLOOR_DB = -60;
export const FALL_DB_PER_S = 24;
export const PEAK_HOLD_MS = 1500;
export const PEAK_FALL_DB_PER_S = 18;
/** A reading at or above this (sample or true peak) lights the clip indicator. */
export const CLIP_DB = 0;

export interface ChannelState {
  /** Bar level, dB. */
  level: number;
  /** Peak marker, dB, and when it was last raised (ms). */
  peak: number;
  peakAt: number;
  clip: boolean;
}

export interface MeterState {
  channels: [ChannelState, ChannelState];
  /** Loudness of the latest reading, LUFS (null when not available). */
  momentary: number | null;
  shortTerm: number | null;
  /** Highest true peak since the meters were reset, dBTP. */
  truePeakMax: number;
}

export function idleChannel(): ChannelState {
  return { level: METER_FLOOR_DB, peak: METER_FLOOR_DB, peakAt: -Infinity, clip: false };
}

export function idleMeters(): MeterState {
  return { channels: [idleChannel(), idleChannel()], momentary: null, shortTerm: null, truePeakMax: METER_FLOOR_DB };
}

/** One channel after `dtMs` with input `db` (null: no new reading, the bar only falls). */
export function stepChannel(s: ChannelState, db: number | null, clipDb: number | null, dtMs: number, nowMs: number): ChannelState {
  const fallen = Math.max(METER_FLOOR_DB, s.level - (FALL_DB_PER_S * dtMs) / 1000);
  const input = db === null ? METER_FLOOR_DB : Math.max(METER_FLOOR_DB, db);
  const level = Math.max(fallen, input);
  let peak = s.peak;
  let peakAt = s.peakAt;
  if (input >= peak) {
    peak = input;
    peakAt = nowMs;
  } else if (nowMs - peakAt > PEAK_HOLD_MS) {
    peak = Math.max(level, peak - (PEAK_FALL_DB_PER_S * dtMs) / 1000);
  }
  const clip = s.clip || (clipDb !== null && clipDb >= CLIP_DB);
  return { level, peak, peakAt, clip };
}

/** Joins readings: the highest peaks, the latest loudness. */
export function joinReadings(rs: MeterReadingView[]): MeterReadingView | null {
  if (!rs.length) return null;
  const last = rs[rs.length - 1];
  const max = (k: "peak" | "rms" | "truePeak", c: number) => Math.max(...rs.map((r) => r[k][c] ?? METER_FLOOR_DB));
  return {
    start: rs[0].start,
    end: last.end,
    peak: [max("peak", 0), max("peak", 1)],
    rms: [max("rms", 0), max("rms", 1)],
    truePeak: [max("truePeak", 0), max("truePeak", 1)],
    momentary: last.momentary ?? null,
    shortTerm: last.shortTerm ?? null,
  };
}

/** The meters after `dtMs`, given the readings heard since the last step. */
export function stepMeters(s: MeterState, rs: MeterReadingView[], dtMs: number, nowMs: number): MeterState {
  const r = joinReadings(rs);
  const ch = (c: 0 | 1) =>
    stepChannel(s.channels[c], r ? (r.peak[c] ?? null) : null, r ? Math.max(r.peak[c] ?? -Infinity, r.truePeak[c] ?? -Infinity) : null, dtMs, nowMs);
  return {
    channels: [ch(0), ch(1)],
    momentary: r ? (r.momentary ?? null) : s.momentary,
    shortTerm: r ? (r.shortTerm ?? null) : s.shortTerm,
    truePeakMax: r ? Math.max(s.truePeakMax, ...r.truePeak.map((v) => v ?? METER_FLOOR_DB)) : s.truePeakMax,
  };
}

/** Clears the clip indicators (the person clicked them). */
export function resetClip(s: MeterState): MeterState {
  return { ...s, channels: [{ ...s.channels[0], clip: false }, { ...s.channels[1], clip: false }], truePeakMax: METER_FLOOR_DB };
}

/** Follows the engine's batches: what is heard now, and the readings heard since the last call. */
export class HeardReadings {
  private batch: AudioMeter | null = null;
  private receivedAt = 0;
  private taken = -Infinity;

  /** A new batch arrived at `nowMs`. */
  receive(b: AudioMeter, nowMs: number) {
    this.batch = b;
    this.receivedAt = nowMs;
    // Playback (re)starts: take from just before the heard position of the first batch.
    if (!b.playing) this.taken = -Infinity;
    else if (this.taken === -Infinity) this.taken = (b.heard ?? 0) - 0.02;
  }

  /** Media seconds heard at `nowMs` (null when not playing). */
  heard(nowMs: number): number | null {
    const b = this.batch;
    if (!b || !b.playing) return null;
    return (b.heard ?? 0) + (nowMs - this.receivedAt) / 1000;
  }

  /** Readings that started since the last call and are heard by `nowMs`. */
  take(nowMs: number): MeterReadingView[] {
    const h = this.heard(nowMs);
    if (h === null || !this.batch) return [];
    // After a jump back (seek, loop) start over.
    if (h < this.taken - 0.1) this.taken = h - 0.02;

    const from = this.taken === -Infinity ? h - 0.02 : this.taken;
    const out = this.batch.readings.filter((r) => (r.start ?? 0) > from && (r.start ?? 0) <= h);
    if (out.length) this.taken = out[out.length - 1].start ?? this.taken;
    return out;
  }
}
