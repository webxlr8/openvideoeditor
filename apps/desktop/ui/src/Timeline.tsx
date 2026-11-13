// The timeline: real tracks and items of the open project. Click to select (shift/⌘ to add), drag to move (also to
// another compatible track), drag an edge to trim (the start edge moves the in-point), right-click for actions.
// Snapping to the playhead and clip edges is on by default. Every gesture commits one revision on release.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ItemView_Serialize, ProjectView_Serialize } from "./bindings";
import { commands } from "./bridge";
import { type EditResult, addItems, firstTrack, trackAccepts, updateItem, updateItems, snapToFrame } from "./edit";
import { Icon, type IconName } from "./icons";
import { ContextMenu, type MenuItem } from "./Menu";
import { MOD } from "./keys";

export const HEADER_W = 188;
const ROW_H = 41;
const SNAP_PX = 8;
const TRACK_ICON: Record<string, IconName> = { video: "media", caption: "captions", audio: "audio", adjustment: "effects", layout: "layout" };

export type SelectMode = "replace" | "toggle";

export interface TimelineProps {
  project: ProjectView_Serialize | null;
  selection: string[];
  playhead: number;
  /** Pixels per second; null fits the whole timeline. */
  zoom: number | null;
  snap: boolean;
  onSelect: (item: ItemView_Serialize | null, offsetSeconds?: number, mode?: SelectMode) => void;
  onScrub: (t: number) => void;
  onEdit: (p: EditResult) => void;
  onCopy: (ids: string[]) => void;
  onPaste: (at: number) => void;
  canPaste: boolean;
  onFitWidth?: (pxPerSecond: number) => void;
  /** Items to mark (what an assistant turn changed, while the chat's "Show" is on). */
  highlight?: string[];
}

type Drag =
  | { kind: "move"; id: string; dx: number; dy: number; ids: string[] }
  | { kind: "trim"; id: string; edge: "start" | "end"; dt: number };

function fmtTime(t: number) {
  const ms = Math.round(t * 1000);
  const s = Math.floor(ms / 1000);
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}.${String(Math.floor((ms % 1000) / 10)).padStart(2, "0")}`;
}

function fmtTick(s: number) {
  return s >= 60 ? `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}` : `${Math.round(s * 10) / 10}s`;
}

export function Timeline(props: TimelineProps) {
  const { project, selection, playhead, zoom, snap, onSelect, onScrub, onEdit, onCopy, onPaste, canPaste, onFitWidth, highlight } = props;
  const scroller = useRef<HTMLDivElement>(null);
  const rows = useRef<HTMLDivElement>(null);
  const [viewW, setViewW] = useState(1000);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; items: MenuItem[] } | null>(null);
  const items = project?.items ?? [];
  const tracks = project?.tracks ?? [];

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setViewW(el.clientWidth || 1000));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const total = Math.max(10, (project?.durationSeconds ?? 0) * 1.15, playhead + 1);
  const fitPps = Math.max(1, viewW - HEADER_W) / total;
  const pps = zoom ?? fitPps;
  const contentW = Math.max(viewW, HEADER_W + total * pps);
  const duration = (contentW - HEADER_W) / pps;

  useEffect(() => {
    onFitWidth?.(fitPps);
  }, [fitPps, onFitWidth]);

  const step = (() => {
    const minPx = 70;
    for (const s of [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600]) if (s * pps >= minPx) return s;
    return 1200;
  })();
  const ticks: number[] = [];
  for (let s = 0; s <= duration; s += step) ticks.push(Math.round(s * 1000) / 1000);

  const x = (t: number) => t * pps;
  const timeAt = (clientX: number) => {
    const r = scroller.current?.getBoundingClientRect();
    if (!r) return 0;
    return Math.max(0, (clientX - r.left + (scroller.current?.scrollLeft ?? 0) - HEADER_W) / pps);
  };

  /** Snaps a time to the playhead, 0 and other items' edges (within SNAP_PX). */
  const snapTime = (t: number, ignore: string[]) => {
    if (!snap) return t;
    const points = [0, playhead, ...items.filter((i) => !ignore.includes(i.id)).flatMap((i) => [i.start ?? 0, i.end ?? 0])];
    let best = t;
    let bestD = SNAP_PX / pps;
    for (const p of points) {
      const d = Math.abs(p - t);
      if (d < bestD) {
        best = p;
        bestD = d;
      }
    }
    return best;
  };

  const seekAt = (clientX: number) => {
    const t = timeAt(clientX);
    const order = new Map(tracks.map((tr, i) => [tr.id, i]));
    // The topmost visual clip under the playhead is previewed (sequence playback arrives with the compositor).
    const hit = items
      .filter((i) => i.mediaId && (i.start ?? 0) <= t && t < (i.end ?? 0))
      .sort((a, b) => (order.get(a.trackId) ?? 0) - (order.get(b.trackId) ?? 0))[0];
    if (hit) onSelect(hit, t - (hit.start ?? 0));
    else onScrub(t);
  };

  const trackAtY = (clientY: number) => {
    const r = rows.current?.getBoundingClientRect();
    if (!r) return null;
    const i = Math.floor((clientY - r.top) / ROW_H);
    return tracks[Math.min(Math.max(i, 0), tracks.length - 1)] ?? null;
  };

  const startMove = (e: React.PointerEvent, item: ItemView_Serialize) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    if (additive) {
      onSelect(item, undefined, "toggle");
      return;
    }
    const group = selection.includes(item.id) && selection.length > 1 ? selection : [item.id];
    // The playhead goes to the clicked point (not the clip's start), so "split at playhead" splits where clicked.
    if (!selection.includes(item.id) || selection.length <= 1) onSelect(item, Math.max(0, timeAt(e.clientX) - (item.start ?? 0)));
    const x0 = e.clientX;
    const y0 = e.clientY;
    let last: Drag | null = null;
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - x0;
      const dy = ev.clientY - y0;
      if (Math.abs(dx) > 3 || Math.abs(dy) > 6) {
        last = { kind: "move", id: item.id, dx, dy, ids: group };
        setDrag(last);
      }
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      if (!last) return;
      const raw = (item.start ?? 0) + (ev.clientX - x0) / pps;
      // Snap the start, or the end if that is closer to a snap point.
      const len = (item.end ?? 0) - (item.start ?? 0);
      const s1 = snapTime(raw, group);
      const s2 = snapTime(raw + len, group) - len;
      let start = Math.abs(s1 - raw) <= Math.abs(s2 - raw) ? s1 : s2;
      start = snapToFrame(start);
      const delta = start - (item.start ?? 0);
      const target = trackAtY(ev.clientY);
      const changeTrack = group.length === 1 && target && target.id !== item.trackId && trackAccepts(target.kind, item.kind) ? target.id : null;
      if (group.length > 1) {
        const edits = items
          .filter((i) => group.includes(i.id))
          .map((i) => {
            const s = Math.max(0, (i.start ?? 0) + delta);
            return { id: i.id, set: { startTime: s, endTime: s + ((i.end ?? 0) - (i.start ?? 0)) } };
          });
        onEdit(updateItems(edits, `move ${edits.length} items`));
      } else if (Math.abs(delta) > 1e-6 || changeTrack) {
        onEdit(commands.moveItem(item.id, start, changeTrack));
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const startTrim = (e: React.PointerEvent, item: ItemView_Serialize, edge: "start" | "end") => {
    if (e.button !== 0) return;
    e.stopPropagation();
    onSelect(item);
    const x0 = e.clientX;
    let moved = false;
    const move = (ev: PointerEvent) => {
      moved = true;
      setDrag({ kind: "trim", id: item.id, edge, dt: (ev.clientX - x0) / pps });
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      if (!moved) return;
      const base = edge === "start" ? (item.start ?? 0) : (item.end ?? 0);
      const t = snapTime(base + (ev.clientX - x0) / pps, [item.id]);
      onEdit(commands.trimItem(item.id, edge, snapToFrame(t)));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const clipMenu = (e: React.MouseEvent, item: ItemView_Serialize) => {
    e.preventDefault();
    e.stopPropagation();
    if (!selection.includes(item.id)) onSelect(item);
    const ids = selection.includes(item.id) ? selection : [item.id];
    const inside = playhead > (item.start ?? 0) && playhead < (item.end ?? 0);
    const sound = item.kind === "video" || item.kind === "audio";
    const list: MenuItem[] = [
      ...(inside ? [{ id: "split", label: "Split at playhead", hint: "S", onSelect: () => onEdit(commands.splitItem(item.id, playhead)) }] : []),
      { id: "duplicate", label: "Duplicate", hint: "D", onSelect: () => onEdit(commands.duplicateItems(ids)) },
      { id: "copy", label: "Copy", hint: `${MOD} C`, onSelect: () => onCopy(ids) },
      ...(canPaste ? [{ id: "paste", label: "Paste at playhead", hint: `${MOD} V`, onSelect: () => onPaste(playhead) }] : []),
      ...(sound ? [{ id: "mute", label: item.muted ? "Unmute" : "Mute", onSelect: () => onEdit(updateItem(item.id, { muted: !item.muted }, [], item.muted ? "unmute" : "mute")) }] : []),
      { id: "hide", label: item.hidden ? "Show" : "Hide", onSelect: () => onEdit(updateItem(item.id, { hidden: !item.hidden }, [], item.hidden ? "show" : "hide")) },
      { id: "delete", label: ids.length > 1 ? `Delete ${ids.length} items` : "Delete", hint: "⌫", danger: true, onSelect: () => onEdit(commands.removeItems(ids)) },
    ];
    setMenu({ at: { x: e.clientX, y: e.clientY }, items: list });
  };

  const laneMenu = (e: React.MouseEvent, trackId: string, trackKind: string) => {
    e.preventDefault();
    const t = snapToFrame(timeAt(e.clientX));
    const list: MenuItem[] = [];
    if (canPaste) list.push({ id: "paste", label: "Paste here", onSelect: () => onPaste(t) });
    if (trackKind === "video")
      list.push({
        id: "add-text",
        label: "Add text here",
        onSelect: () =>
          onEdit(addItems([{ trackId, type: "text", startTime: t, endTime: t + 3, name: "Text", textContent: "Your text", fontSize: 96, fontWeight: 700, color: "#ffffff" }], "add text")),
      });
    if (trackKind === "caption")
      list.push({
        id: "add-caption",
        label: "Add caption here",
        onSelect: () =>
          onEdit(
            addItems(
              [{ trackId, type: "caption", startTime: t, endTime: t + 3, name: "Caption", captionText: "Your words here", captionSegments: [{ id: "s1", text: "Your words here", startTime: 0, endTime: 3 }] }],
              "add caption",
            ),
          ),
      });
    if (list.length === 0) list.push({ id: "none", label: "Nothing to add here" });
    setMenu({ at: { x: e.clientX, y: e.clientY }, items: list });
  };

  const geometry = (i: ItemView_Serialize) => {
    let start = i.start ?? 0;
    let end = i.end ?? 0;
    let top = 0;
    if (drag?.kind === "move" && drag.ids.includes(i.id)) {
      start += drag.dx / pps;
      end += drag.dx / pps;
      if (drag.id === i.id && drag.ids.length === 1) top = drag.dy;
    }
    if (drag?.kind === "trim" && drag.id === i.id) {
      if (drag.edge === "start") start = Math.min(end - 0.04, Math.max(0, start + drag.dt));
      else end = Math.max(start + 0.04, end + drag.dt);
    }
    return { left: x(Math.max(0, start)), width: Math.max(4, x(end - Math.max(0, start))), top };
  };

  return (
    <div className="timeline" data-testid="timeline">
      <div className="tl-scroll" ref={scroller}>
        <div className="tl-content" style={{ width: contentW }}>
          <div className="ruler" onPointerDown={(e) => seekAt(e.clientX)} data-testid="ruler">
            <div className="ruler-head small">{fmtTime(playhead)}</div>
            <div className="ruler-ticks">
              {ticks.map((s) => (
                <span key={s} style={{ left: x(s) }}>
                  {fmtTick(s)}
                </span>
              ))}
            </div>
          </div>
          <div className="tracks" ref={rows}>
            {tracks.map((t) => (
              <div className={`track${t.visible ? "" : " track-hidden"}`} key={t.id} data-testid={`track-${t.name}`}>
                <div className="track-head">
                  <Icon name={TRACK_ICON[t.kind] ?? "media"} size={14} />
                  <span>{t.name}</span>
                  <span className="track-tools">
                    {t.kind !== "caption" && (
                      <button
                        className={`tiny${t.muted ? " on" : ""}`}
                        title={t.muted ? "Unmute track" : "Mute track"}
                        onClick={() => onEdit(commands.setTrackFlag(t.id, "muted", !t.muted))}
                      >
                        M
                      </button>
                    )}
                    <button
                      className={`tiny${t.visible ? "" : " on"}`}
                      aria-label="Visibility"
                      title={t.visible ? "Hide track" : "Show track"}
                      onClick={() => onEdit(commands.setTrackFlag(t.id, "visible", !t.visible))}
                    >
                      <Icon name="eye" size={14} />
                    </button>
                  </span>
                </div>
                <div
                  className="track-lane"
                  data-testid={`lane-${t.name}`}
                  onPointerDown={(e) => (e.target === e.currentTarget && e.button === 0 ? seekAt(e.clientX) : undefined)}
                  onContextMenu={(e) => laneMenu(e, t.id, t.kind)}
                >
                  {items
                    .filter((i) => i.trackId === t.id)
                    .map((i) => {
                      const g = geometry(i);
                      const thumb = project?.media.find((m) => m.id === i.mediaId)?.thumbnail;
                      const sel = selection.includes(i.id);
                      return (
                        <div
                          key={i.id}
                          className={`clip kind-${i.kind}${sel ? " selected" : ""}${highlight?.includes(i.id) ? " reviewed" : ""}${i.pending ? " pending" : ""}${i.hidden ? " is-hidden" : ""}${
                            drag && "id" in drag && drag.id === i.id ? " dragging" : ""
                          }`}
                          style={{ left: g.left, width: g.width, transform: g.top ? `translateY(${g.top}px)` : undefined, backgroundImage: thumb ? `url(${thumb})` : undefined }}
                          title={`${i.name} · ${fmtTime(i.start ?? 0)}–${fmtTime(i.end ?? 0)}`}
                          data-testid="clip"
                          data-id={i.id}
                          onPointerDown={(e) => startMove(e, i)}
                          onContextMenu={(e) => clipMenu(e, i)}
                        >
                          {(i.transitionIn ?? 0) > 0 && <span className="fade-in" style={{ width: x(i.transitionIn ?? 0) }} />}
                          {(i.transitionOut ?? 0) > 0 && <span className="fade-out" style={{ width: x(i.transitionOut ?? 0) }} />}
                          <span className="clip-label">
                            {i.muted && <Icon name="mute" size={11} />} {i.kind === "text" || i.kind === "caption" ? (i.text ?? i.name) : i.name}
                            {(i.speed ?? 1) !== 1 && <em className="speed-tag"> {i.speed}×</em>}
                          </span>
                          {sel && (
                            <>
                              <span className="trim trim-start" data-testid="trim-start" onPointerDown={(e) => startTrim(e, i, "start")} />
                              <span className="trim trim-end" data-testid="trim-end" onPointerDown={(e) => startTrim(e, i, "end")} />
                            </>
                          )}
                        </div>
                      );
                    })}
                </div>
              </div>
            ))}
            {!items.length && <div className="timeline-empty muted">Import media to start your timeline.</div>}
          </div>
          <div className="playhead" style={{ left: HEADER_W + x(playhead) }} data-testid="playhead" />
        </div>
      </div>
      <ContextMenu at={menu?.at ?? null} items={menu?.items ?? []} onClose={() => setMenu(null)} testId="clip-menu" />
    </div>
  );
}

/** First track that can hold an item kind (for adding items from panels). */
export function trackFor(p: ProjectView_Serialize, kind: string): string | null {
  if (kind === "text") return firstTrack(p, "video", "t_text");
  if (kind === "caption") return firstTrack(p, "caption", "t_captions");
  if (kind === "audio") return firstTrack(p, "audio", "t_audio");
  return firstTrack(p, "video", "t_main");
}
