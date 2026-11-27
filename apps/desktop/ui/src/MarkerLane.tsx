// The marker lane under the ruler: markers as flags (ranges as bars), a status filter, and a popover editor on click.
// The filter is view state; adding (M), saving and deleting are each one revision.

import { useState } from "react";
import "./markers.css";
import type { MarkerView_Serialize as MarkerView } from "./bindings";
import { commands } from "./bridge";
import type { EditResult } from "./edit";

export const MARKER_STATUSES = ["open", "review", "resolved"] as const;
export type MarkerFilter = "all" | (typeof MARKER_STATUSES)[number];

/** Named marker colours (any #rrggbb value also works). */
export const MARKER_COLORS: Record<string, string> = {
  red: "#e5484d",
  orange: "#f76b15",
  yellow: "#ffc53d",
  green: "#30a46c",
  teal: "#12a594",
  blue: "#0090ff",
  purple: "#8e4ec6",
  pink: "#d6409f",
};

export function markerColor(c: string | null | undefined): string {
  if (!c) return MARKER_COLORS.yellow;
  return MARKER_COLORS[c] ?? (/^#[0-9a-f]{6}$/i.test(c) ? c : MARKER_COLORS.yellow);
}

export interface MarkerLaneProps {
  markers: MarkerView[];
  /** Pixels per second. */
  pps: number;
  headerW: number;
  onEdit: (p: EditResult) => void;
  onSeek: (t: number) => void;
}

export function MarkerLane({ markers, pps, headerW, onEdit, onSeek }: MarkerLaneProps) {
  const [filter, setFilter] = useState<MarkerFilter>("all");
  const [open, setOpen] = useState<string | null>(null);
  const shown = markers.filter((m) => filter === "all" || m.status === filter);
  const editing = markers.find((m) => m.id === open) ?? null;
  return (
    <div className="marker-lane" data-testid="marker-lane">
      <div className="marker-lane-head" style={{ width: headerW }}>
        <span className="small">Markers</span>
        <select
          className="marker-filter"
          aria-label="Show markers"
          data-testid="marker-filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value as MarkerFilter)}
        >
          <option value="all">All</option>
          <option value="open">Open</option>
          <option value="review">Review</option>
          <option value="resolved">Resolved</option>
        </select>
      </div>
      <div className="marker-lane-body" style={{ left: headerW }}>
        {shown.map((m) => {
          const at = m.at ?? 0;
          const dur = m.dur ?? 0;
          return (
            <button
              key={m.id}
              type="button"
              className={`marker marker-${m.status}${dur > 0 ? " marker-range" : ""}${open === m.id ? " marker-open" : ""}`}
              data-testid="marker"
              data-id={m.id}
              data-status={m.status}
              style={{ left: at * pps, width: dur > 0 ? Math.max(6, dur * pps) : undefined, ["--marker-color" as string]: markerColor(m.color) }}
              title={[m.name, m.note, m.status, m.assignee && `for ${m.assignee}`].filter(Boolean).join(" · ")}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={() => {
                onSeek(at);
                setOpen(m.id);
              }}
            >
              <span className="marker-flag" />
              {m.name && <span className="marker-label">{m.name}</span>}
            </button>
          );
        })}
      </div>
      {editing && (
        <MarkerPopover
          key={editing.id}
          marker={editing}
          left={headerW + (editing.at ?? 0) * pps}
          onEdit={onEdit}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  );
}

export interface MarkerPopoverProps {
  marker: MarkerView;
  left: number;
  onEdit: (p: EditResult) => void;
  onClose: () => void;
}

/** The fields a save changes: set for new values, unset for cleared optional fields. */
export function markerChanges(
  m: MarkerView,
  draft: { name: string; note: string; color: string; status: string; assignee: string },
): { set: Record<string, string>; unset: string[] } {
  const set: Record<string, string> = {};
  const unset: string[] = [];
  for (const k of ["name", "note", "color", "assignee"] as const) {
    const was = m[k] ?? "";
    const now = draft[k].trim();
    if (now === was) continue;
    if (now) set[k] = now;
    else unset.push(k);
  }
  if (draft.status !== m.status) set.status = draft.status;
  return { set, unset };
}

export function MarkerPopover({ marker, left, onEdit, onClose }: MarkerPopoverProps) {
  const [draft, setDraft] = useState({
    name: marker.name ?? "",
    note: marker.note ?? "",
    color: marker.color ?? "",
    status: marker.status,
    assignee: marker.assignee ?? "",
  });
  const field = (k: keyof typeof draft) => (e: { target: { value: string } }) => setDraft((d) => ({ ...d, [k]: e.target.value }));
  const save = () => {
    const { set, unset } = markerChanges(marker, draft);
    if (Object.keys(set).length || unset.length) onEdit(commands.markerUpdate(marker.id, JSON.stringify(set), unset));
    onClose();
  };
  const colorChoices = Object.keys(MARKER_COLORS);
  return (
    <div
      className="marker-popover"
      role="dialog"
      aria-label="Edit marker"
      data-testid="marker-popover"
      style={{ left }}
      onPointerDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
        e.stopPropagation();
      }}
    >
      <label className="marker-row">
        <span>Name</span>
        <input value={draft.name} onChange={field("name")} data-testid="marker-name" />
      </label>
      <label className="marker-row">
        <span>Note</span>
        <textarea value={draft.note} onChange={field("note")} maxLength={4000} rows={3} data-testid="marker-note" />
      </label>
      <label className="marker-row">
        <span>Colour</span>
        <select value={draft.color} onChange={field("color")} data-testid="marker-color">
          <option value="">Default</option>
          {colorChoices.map((c) => (
            <option key={c} value={c}>
              {c[0].toUpperCase() + c.slice(1)}
            </option>
          ))}
          {draft.color && !colorChoices.includes(draft.color) && <option value={draft.color}>{draft.color}</option>}
        </select>
      </label>
      <label className="marker-row">
        <span>Status</span>
        <select value={draft.status} onChange={field("status")} data-testid="marker-status">
          <option value="open">Open</option>
          <option value="review">Review</option>
          <option value="resolved">Resolved</option>
        </select>
      </label>
      <label className="marker-row">
        <span>For</span>
        <input value={draft.assignee} onChange={field("assignee")} placeholder="agent, user or a name" data-testid="marker-assignee" />
      </label>
      <div className="marker-actions">
        <button
          type="button"
          className="danger"
          data-testid="marker-delete"
          onClick={() => {
            onEdit(commands.markerRemove([marker.id]));
            onClose();
          }}
        >
          Delete
        </button>
        <span className="spacer" />
        <button type="button" onClick={onClose} data-testid="marker-cancel">
          Cancel
        </button>
        <button type="button" className="primary" onClick={save} data-testid="marker-save">
          Save
        </button>
      </div>
    </div>
  );
}
