// Keyframe lanes under an expanded track: one lane per keyframed property of the track's items (volume keeps its
// gain line on the clip). Diamonds drag in time and value; Alt-click (Option-click on macOS) on the lane adds a
// keyframe at the current value; Delete removes the focused diamond. Each gesture is one revision; expanding a track
// is view state only.

import { useState } from "react";
import "./keyframes.css";
import type { ItemView_Serialize, KeyView_Serialize } from "./bindings";
import { commands } from "./bridge";
import type { EditResult } from "./edit";

export const LANE_H = 24;
const PAD = 6;

/** Lane order and labels: the order of the keyframe properties in the document schema. */
export const KEYFRAME_LABELS: Record<string, string> = {
  x: "Position X",
  y: "Position Y",
  scale: "Scale",
  rotation: "Rotation",
  opacity: "Opacity",
  gaussianBlur: "Blur",
  borderRadius: "Radius",
  "crop.top": "Crop top",
  "crop.right": "Crop right",
  "crop.bottom": "Crop bottom",
  "crop.left": "Crop left",
};
const ORDER = Object.keys(KEYFRAME_LABELS);

type Key = { time: number; value: number; easing: string | null };

const keysOf = (keys: KeyView_Serialize[]): Key[] => keys.map((k) => ({ time: k.time ?? 0, value: k.value ?? 0, easing: k.easing }));

/** The keyframed properties of a track's items, in schema order. */
export function laneProperties(items: ItemView_Serialize[]): string[] {
  const found = new Set(items.flatMap((i) => (i.keyframes ?? []).map((t) => t.property)));
  return [...found].sort((a, b) => (ORDER.indexOf(a) + 1 || 99) - (ORDER.indexOf(b) + 1 || 99) || a.localeCompare(b));
}

/** Easing curves, as the renderer evaluates them. */
export function ease(easing: string, p: number): number {
  const t = Math.min(1, Math.max(0, p));
  switch (easing) {
    case "ease-in":
      return t * t;
    case "ease-out":
      return 1 - (1 - t) * (1 - t);
    case "ease-in-out":
      return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
    case "hold":
      return t >= 1 ? 1 : 0;
    default:
      return t;
  }
}

/** Value of sorted keys at item-relative time `t` (each segment eases with its start key's easing, else the track's). */
export function valueAt(keys: Key[], trackEasing: string, t: number): number {
  if (!keys.length) return 0;
  if (t <= keys[0].time) return keys[0].value;
  const last = keys[keys.length - 1];
  if (t >= last.time) return last.value;
  const i = keys.findIndex((k) => k.time > t);
  const a = keys[i - 1];
  const b = keys[i];
  const p = (t - a.time) / Math.max(1e-6, b.time - a.time);
  return a.value + (b.value - a.value) * ease(a.easing ?? trackEasing, p);
}

export interface KeyframeLanesProps {
  items: ItemView_Serialize[];
  /** Pixels per second. */
  pps: number;
  headerW: number;
  onEdit: (p: EditResult) => void;
}

type Drag = { item: string; index: number; dx: number; dy: number };

export function KeyframeLanes({ items, pps, headerW, onEdit }: KeyframeLanesProps) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const props = laneProperties(items);
  const save = (item: ItemView_Serialize, property: string, keys: Key[], summary: string) => {
    const sorted = [...keys].sort((a, b) => a.time - b.time);
    onEdit(commands.keyframesSet(item.id, property, JSON.stringify(sorted.map((k) => (k.easing ? k : { time: k.time, value: k.value }))), summary));
  };
  return (
    <div className="kf-lanes" data-testid="kf-lanes">
      {props.map((property) => {
        const tracks = items.flatMap((i) => (i.keyframes ?? []).filter((t) => t.property === property).map((t) => ({ item: i, track: t, keys: keysOf(t.keys) })));
        const values = tracks.flatMap((t) => t.keys.map((k) => k.value));
        const lo = Math.min(...values);
        const hi = Math.max(...values);
        const span = hi - lo || Math.max(1, Math.abs(hi));
        const y = (v: number) => LANE_H - PAD - ((v - (hi === lo ? lo - span / 2 : lo)) / span) * (LANE_H - 2 * PAD);
        const valuePerPx = span / (LANE_H - 2 * PAD);
        return (
          <div className="kf-lane" key={property} data-testid={`kf-lane-${property}`}>
            <div className="kf-lane-head small" style={{ width: headerW }}>
              {KEYFRAME_LABELS[property] ?? property}
            </div>
            <div
              className="kf-lane-body"
              data-testid={`kf-body-${property}`}
              onPointerDown={(e) => {
                if (!e.altKey || e.button !== 0) return;
                const r = e.currentTarget.getBoundingClientRect();
                const t = (e.clientX - r.left) / pps;
                const hit = tracks.find((tr) => (tr.item.start ?? 0) <= t && t <= (tr.item.end ?? 0));
                if (!hit) return;
                e.stopPropagation();
                const rel = Math.round((t - (hit.item.start ?? 0)) * 1000) / 1000;
                const value = valueAt(hit.keys, hit.track.easing, rel);
                save(hit.item, property, [...hit.keys.filter((k) => k.time !== rel), { time: rel, value, easing: null }], "add keyframe");
              }}
            >
              {tracks.map(({ item, track, keys }) =>
                keys.map((k, index) => {
                  const d = drag && drag.item === item.id && drag.index === index && track.property === property ? drag : null;
                  const left = ((item.start ?? 0) + k.time) * pps + (d?.dx ?? 0);
                  const top = y(k.value) + (d?.dy ?? 0);
                  return (
                    <button
                      key={`${item.id}-${index}`}
                      type="button"
                      className={`kf-diamond${k.easing === "hold" || (!k.easing && track.easing === "hold") ? " kf-hold" : ""}`}
                      data-testid="kf-diamond"
                      data-item={item.id}
                      data-time={k.time}
                      data-value={k.value}
                      style={{ left, top }}
                      title={`${KEYFRAME_LABELS[property] ?? property} ${Math.round(k.value * 1000) / 1000} at ${Math.round(k.time * 1000) / 1000}s${k.easing ? ` · ${k.easing}` : ""}`}
                      onKeyDown={(e) => {
                        if (e.key !== "Delete" && e.key !== "Backspace") return;
                        e.preventDefault();
                        e.stopPropagation();
                        save(item, property, keys.filter((_, i) => i !== index), "delete keyframe");
                      }}
                      onPointerDown={(e) => {
                        if (e.button !== 0) return;
                        e.stopPropagation();
                        (e.currentTarget as HTMLElement).focus();
                        const x0 = e.clientX;
                        const y0 = e.clientY;
                        let moved = false;
                        const move = (ev: PointerEvent) => {
                          if (Math.abs(ev.clientX - x0) > 2 || Math.abs(ev.clientY - y0) > 2) moved = true;
                          if (moved) setDrag({ item: item.id, index, dx: ev.clientX - x0, dy: ev.clientY - y0 });
                        };
                        const up = (ev: PointerEvent) => {
                          window.removeEventListener("pointermove", move);
                          window.removeEventListener("pointerup", up);
                          setDrag(null);
                          if (!moved) return;
                          const len = (item.end ?? 0) - (item.start ?? 0);
                          const time = Math.round(Math.min(len, Math.max(0, k.time + (ev.clientX - x0) / pps)) * 1000) / 1000;
                          const value = k.value - (ev.clientY - y0) * valuePerPx;
                          const next = keys.map((x, i) => (i === index ? { ...x, time, value } : x)).filter((x, i) => i === index || x.time !== time);
                          save(item, property, next, "move keyframe");
                        };
                        window.addEventListener("pointermove", move);
                        window.addEventListener("pointerup", up);
                      }}
                    />
                  );
                }),
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
