// Timecode display and entry (non-drop-frame, frames counted from 0).

function pad(n: number, width = 2): string {
  return String(Math.trunc(n)).padStart(width, "0");
}

/** Integer timebase used for the frames field (29.97 → 30, 23.976 → 24). */
export function timebase(fps: number | null): number {
  return Math.max(1, Math.round(fps ?? 30));
}

/** `HH:MM:SS:FF` for a frame index. */
export function formatTimecode(frame: number, fps: number | null): string {
  const base = timebase(fps);
  const f = Math.max(0, Math.trunc(frame));
  const totalSeconds = Math.floor(f / base);
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  return `${pad(h)}:${pad(m)}:${pad(s)}:${pad(f % base)}`;
}

/** `m:ss` (or `h:mm:ss`) for a frame index, for compact displays. */
export function formatShort(frame: number, fps: number | null): string {
  const totalSeconds = Math.floor(Math.max(0, frame) / timebase(fps));
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/**
 * Parses user timecode entry into a frame index, clamped to the media.
 * Accepts `HH:MM:SS:FF`, `MM:SS:FF`, `MM:SS`, seconds (`12.5`), or frames (`300f`).
 * Returns null when the text is not a timecode.
 */
export function parseTimecode(text: string, fps: number | null, totalFrames: number): number | null {
  const base = timebase(fps);
  const t = text.trim();
  if (t === "") return null;
  let frame: number;
  const frames = /^(\d+)\s*f$/i.exec(t);
  if (frames) {
    frame = Number(frames[1]);
  } else if (t.includes(":") || t.includes(";")) {
    const parts = t.split(/[:;]/);
    if (parts.length < 2 || parts.length > 4 || parts.some((p) => !/^\d+$/.test(p))) return null;
    const n = parts.map(Number);
    // Right-align the fields: [h, m, s, f] for 4 parts, [m, s, f] for 3, [m, s] for 2.
    const [h, m, s, f] = n.length === 4 ? n : n.length === 3 ? [0, ...n] : [0, ...n, 0];
    if (s >= 60 || f >= base || (n.length === 4 && m >= 60)) return null;
    frame = ((h * 60 + m) * 60 + s) * base + f;
  } else if (/^\d+(\.\d+)?$/.test(t)) {
    frame = Math.round(Number(t) * base);
  } else {
    return null;
  }
  const last = Math.max(0, totalFrames - 1);
  return Math.min(Math.max(0, frame), last);
}

/** `m:ss` for a duration in seconds. */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

/** "just now", "5 min ago", "3 h ago", or a date, for a time in ms since the epoch. */
export function formatWhen(ms: number, now = Date.now()): string {
  const d = Math.max(0, now - ms) / 1000;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return new Date(ms).toLocaleDateString();
}
