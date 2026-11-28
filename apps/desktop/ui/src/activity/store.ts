// What the timeline's activity layer shows (change add-agent-chat-context-and-activity, design D5): per agent, the
// range it reads, the items it reads or changes, and when it last did anything. Events are applied in batches (one
// per 100 ms); everything fades 3 s after an agent's last event; at most 200 items are outlined at once.

import type { AgentActivity_Serialize } from "../bindings";

export const COALESCE_MS = 100;
export const FADE_MS = 3000;
export const MAX_OUTLINES = 200;

export interface AgentState {
  actor: string;
  name: string;
  /** The last event, ms. */
  last: number;
  /** Item id → read (true) or changed (false); later events win. */
  items: Map<string, "read" | "change">;
  range: { start: number; end: number } | null;
}

export type ActivityState = Map<string, AgentState>;

/** Applies a batch of events (received at `now`). The person's own edits clear those items from every agent. */
export function applyEvents(state: ActivityState, events: AgentActivity_Serialize[], now: number): ActivityState {
  const next: ActivityState = new Map(state);
  for (const e of events) {
    if (e.kind === "person") {
      const gone = [...e.ids, ...e.removed];
      for (const [k, a] of next) {
        if (!gone.some((id) => a.items.has(id))) continue;
        const items = new Map(a.items);
        for (const id of gone) items.delete(id);
        next.set(k, { ...a, items });
      }
      continue;
    }
    const prev = next.get(e.actor);
    const items = new Map(prev?.items ?? []);
    for (const id of e.removed) items.delete(id);
    for (const id of e.ids) {
      // A read does not hide a change made in the same burst.
      if (e.kind === "read" && items.get(id) === "change") continue;
      items.delete(id);
      items.set(id, e.kind === "change" ? "change" : "read");
    }
    const range = e.start != null && e.end != null ? { start: e.start, end: e.end } : (prev?.range ?? null);
    next.set(e.actor, { actor: e.actor, name: e.name, last: now, items, range });
  }
  return next;
}

/** Drops agents idle for [`FADE_MS`]. Returns the same map when nothing faded. */
export function prune(state: ActivityState, now: number): ActivityState {
  let changed = false;
  const next: ActivityState = new Map();
  for (const [k, a] of state) {
    if (now - a.last >= FADE_MS) changed = true;
    else next.set(k, a);
  }
  return changed ? next : state;
}

/** When the next agent fades, ms (none when nothing shows). */
export function nextFade(state: ActivityState): number | null {
  let t: number | null = null;
  for (const a of state.values()) t = t === null ? a.last + FADE_MS : Math.min(t, a.last + FADE_MS);
  return t;
}

export interface Outline {
  id: string;
  kind: "read" | "change";
  actor: string;
}

/** The items to outline, newest first, at most [`MAX_OUTLINES`], and how many more there are. */
export function outlines(state: ActivityState, exists: (id: string) => boolean): { shown: Outline[]; more: number } {
  const all: Outline[] = [];
  const seen = new Set<string>();
  const agents = [...state.values()].sort((a, b) => b.last - a.last);
  for (const a of agents) {
    // Newest first within an agent (a map keeps insertion order; re-touched items move to the end).
    const ids = [...a.items.entries()].reverse();
    for (const [id, kind] of ids) {
      if (seen.has(id) || !exists(id)) continue;
      seen.add(id);
      all.push({ id, kind, actor: a.actor });
    }
  }
  return { shown: all.slice(0, MAX_OUTLINES), more: Math.max(0, all.length - MAX_OUTLINES) };
}
