// The timeline's live agent activity (change add-agent-chat-context-and-activity, design D5): a band on the ruler
// over the time an agent reads, outlines on the items it reads or changes, and a badge with its name. Events from the
// in-app assistant and from external agents (through the running app) arrive as `agentActivity`; they are drawn in
// batches every 100 ms and fade 3 s after the agent's last event. The timeline only mounts this.

import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { AgentActivity_Serialize, ItemView_Serialize, TrackView } from "../bindings";
import { events } from "../bridge";
import { unlisten } from "../notify";
import { type ActivityState, COALESCE_MS, applyEvents, nextFade, outlines, prune } from "./store";
import "./activity.css";

export function ActivityLayer({
  items,
  tracks,
  x,
  headerW,
  rowH,
  rulerH,
}: {
  items: ItemView_Serialize[];
  tracks: TrackView[];
  /** Seconds to pixels from the start of the lanes. */
  x: (t: number) => number;
  headerW: number;
  rowH: number;
  /** Height of the ruler, where the tracks start. */
  rulerH: number;
}) {
  const [state, setState] = useState<ActivityState>(() => new Map());
  const pending = useRef<AgentActivity_Serialize[]>([]);
  const flushTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const un = events.agentActivity.listen((e) => {
      pending.current.push(e.payload);
      if (flushTimer.current !== null) return;
      flushTimer.current = setTimeout(() => {
        flushTimer.current = null;
        const batch = pending.current;
        pending.current = [];
        setState((s) => applyEvents(s, batch, Date.now()));
      }, COALESCE_MS);
    });
    return () => {
      unlisten(un);
      if (flushTimer.current !== null) clearTimeout(flushTimer.current);
      flushTimer.current = null;
    };
  }, []);

  // Fade: wake when the next agent goes idle.
  useEffect(() => {
    const at = nextFade(state);
    if (at === null) return;
    const t = setTimeout(() => setState((s) => prune(s, Date.now())), Math.max(0, at - Date.now()) + 5);
    return () => clearTimeout(t);
  }, [state]);

  const byId = useMemo(() => new Map(items.map((i) => [i.id, i])), [items]);
  const row = useMemo(() => new Map(tracks.map((t, n) => [t.id, n])), [tracks]);
  if (state.size === 0) return null;
  const { shown: newest, more } = outlines(state, (id) => byId.has(id));
  // Drawn in a stable order (they are placed absolutely), so a batch moves no existing outline in the DOM.
  const shown = [...newest].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const agents = [...state.values()].sort((a, b) => b.last - a.last);
  return (
    <>
      {agents
        .filter((a) => a.range)
        .map((a) => (
          <div
            key={`band-${a.actor}`}
            className="activity-band"
            data-testid="activity-band"
            data-start={a.range!.start}
            data-end={a.range!.end}
            title={`${a.name} is reading ${a.range!.start.toFixed(2)}–${a.range!.end.toFixed(2)} s`}
            style={{ left: headerW + x(a.range!.start), width: Math.max(3, x(a.range!.end) - x(a.range!.start)), height: rulerH - 1 }}
          />
        ))}
      {shown.map((o) => {
        const i = byId.get(o.id)!;
        const n = row.get(i.trackId) ?? 0;
        const start = i.start ?? 0;
        return (
          <OutlineBox
            key={o.id}
            id={o.id}
            kind={o.kind}
            left={headerW + x(start)}
            width={Math.max(4, x((i.end ?? 0) - start))}
            top={rulerH + n * rowH + 2}
            height={rowH - 5}
          />
        );
      })}
      <div className="activity-badges">
        <div className="activity-badges-inner" style={{ left: headerW + 6 }}>
          {agents.map((a, n) => {
            const changes = [...a.items.values()].filter((k) => k === "change").length;
            const reads = a.items.size - changes;
            const what = [changes ? `changing ${changes}` : "", reads ? `reading ${reads}` : "", !a.items.size && a.range ? "reading" : ""]
              .filter(Boolean)
              .join(", ");
            return (
              <span className="activity-badge" key={a.actor} data-testid="activity-badge" data-actor={a.actor}>
                <span className="activity-dot" />
                {a.name}
                {what && <span className="muted"> · {what}</span>}
                {n === 0 && more > 0 && (
                  <span className="muted" data-testid="activity-more">
                    {" "}
                    · +{more} more
                  </span>
                )}
              </span>
            );
          })}
        </div>
      </div>
    </>
  );
}

/** One outline; redrawn only when its own box changes (a flood moves few of them per batch). */
const OutlineBox = memo(function OutlineBox({
  id,
  kind,
  left,
  width,
  top,
  height,
}: {
  id: string;
  kind: string;
  left: number;
  width: number;
  top: number;
  height: number;
}) {
  return (
    <div
      className={`activity-outline ${kind}`}
      data-testid="activity-outline"
      data-id={id}
      data-kind={kind}
      style={{ left, width, top, height }}
    />
  );
});
