// Mentions in the chat input (change add-agent-chat-context-and-activity, design D1): what the person can point the
// assistant at with @ — media, timeline items and the marked range — and the `/` menu's skills and templates. The
// app resolves each mention when the message is sent, to the exact printed lines and the revision they came from.

import type { ProjectView_Serialize } from "../bindings";

export type MentionKind = "item" | "range" | "media" | "skill" | "track";

/** A mention in the composer (and the shape panels hand to `askAssistant`). */
export interface MentionDraft {
  kind: MentionKind;
  /** Item, media or skill id (none for a range). */
  id?: string;
  /** A range's start and end, seconds. */
  start?: number;
  end?: number;
  /** What the chip shows. */
  label: string;
  /** For media and items: video, image, audio, text or caption (the popover's tabs use it). */
  media?: string;
  /** A range that stands for the marked in and out points, filled in by the chat (dropped when none is marked). */
  marked?: boolean;
}

export type MentionTab = "all" | "video" | "image" | "audio";

export const MENTION_TABS: { id: MentionTab; label: string }[] = [
  { id: "all", label: "All" },
  { id: "video", label: "Video" },
  { id: "image", label: "Image" },
  { id: "audio", label: "Audio" },
];

/** Most rows the popover lists. */
export const MENTION_ROWS = 30;

function secs(t: number): string {
  return `${t.toFixed(t % 1 === 0 ? 0 : 2)}s`;
}

/** The key that tells two mentions apart. */
export function mentionKey(m: MentionDraft): string {
  return m.kind === "range" ? `range:${m.start}-${m.end}` : `${m.kind}:${m.id}`;
}

/** A range mention. */
export function rangeMention(start: number, end: number): MentionDraft {
  const [a, b] = start <= end ? [start, end] : [end, start];
  return { kind: "range", start: a, end: b, label: `${secs(a)}–${secs(b)}` };
}

/** A track mention (all its items). */
export function trackMention(id: string, name: string): MentionDraft {
  return { kind: "track", id, label: `${name} track` };
}

/** The marked range, as the chat fills it in from the in and out points. */
export const MARKED_RANGE: MentionDraft = { kind: "range", label: "Marked range", marked: true };

/** Resolves a hand-off's marked-range placeholder against the in and out points (none marked: dropped). */
export function fillMarked(ms: MentionDraft[], range?: { in: number | null; out: number | null } | null): MentionDraft[] {
  return ms.flatMap((m) => {
    if (!m.marked) return [m];
    if (!range || range.in === null || range.out === null || range.out <= range.in) return [];
    return [{ ...rangeMention(range.in, range.out), label: `Marked range ${secs(range.in)}–${secs(range.out)}` }];
  });
}

/** What @ offers for a project: the marked range first, then the media, then the timeline items. */
export function mentionCandidates(
  project: ProjectView_Serialize | null | undefined,
  range?: { in: number | null; out: number | null } | null,
): MentionDraft[] {
  const out: MentionDraft[] = [];
  if (range && range.in !== null && range.out !== null && range.out > range.in) {
    out.push({ ...rangeMention(range.in, range.out), label: `Marked range ${secs(range.in)}–${secs(range.out)}` });
  }
  for (const m of project?.media ?? []) out.push({ kind: "media", id: m.id, label: m.name, media: m.kind });
  for (const i of project?.items ?? []) {
    const name = i.kind === "text" || i.kind === "caption" ? (i.text ?? i.name) : i.name;
    out.push({ kind: "item", id: i.id, label: name || i.id, media: i.kind });
  }
  return out;
}

/** The candidates on a tab whose label contains the query (case-insensitive), at most [`MENTION_ROWS`]. */
export function filterMentions(all: MentionDraft[], query: string, tab: MentionTab): MentionDraft[] {
  const q = query.trim().toLowerCase();
  return all
    .filter((m) => tab === "all" || m.media === tab)
    .filter((m) => !q || m.label.toLowerCase().includes(q) || (m.id ?? "").toLowerCase().includes(q))
    .slice(0, MENTION_ROWS);
}

/** The @ token being typed at the end of `before` (the text before the caret), if any. */
export function mentionQuery(before: string): { query: string; at: number } | null {
  const m = before.match(/(^|\s)@([^\s@]*)$/);
  if (!m) return null;
  return { query: m[2], at: before.length - m[2].length - 1 };
}

/** The `/` command being typed: only at the very start of the input, before any space. */
export function slashQuery(text: string): string | null {
  const m = text.match(/^\/(\S*)$/);
  return m ? m[1] : null;
}

/** A row of the `/` menu. */
export interface SlashEntry {
  kind: "skill" | "template";
  id: string;
  label: string;
  hint?: string;
}

/** The `/` entries whose name or id contains the query. */
export function filterSlash(all: SlashEntry[], query: string): SlashEntry[] {
  const q = query.trim().toLowerCase();
  return all.filter((e) => !q || e.label.toLowerCase().includes(q) || e.id.toLowerCase().includes(q));
}

/** Mentions as the app's send command takes them. */
export function mentionInputs(ms: MentionDraft[]) {
  return ms.map((m) => ({ kind: m.kind, id: m.id ?? null, start: m.start ?? null, end: m.end ?? null }));
}
