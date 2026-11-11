// Left-rail tool panels: Text, Captions, Transitions, Effects, Audio, Layout. Each action is one revision.

import { useState } from "react";
import type { ItemView_Serialize, ProjectView_Serialize } from "./bindings";
import { commands } from "./bridge";
import {
  ASPECTS,
  type EditResult,
  LOOK_PRESETS,
  TEXT_PRESETS,
  TRANSITIONS,
  addItems,
  firstTrack,
  nextAdjacent,
  updateItem,
  updateItems,
} from "./edit";
import { Icon } from "./icons";

export interface PanelProps {
  project: ProjectView_Serialize;
  /** Timeline time of the playhead, seconds. */
  playhead: number;
  /** Selected items, primary last. */
  selection: ItemView_Serialize[];
  onEdit: (p: EditResult) => void;
  onSelect: (item: ItemView_Serialize) => void;
}

const round = (t: number) => Math.round(t * 1000) / 1000;
const visual = (i: ItemView_Serialize) => i.kind === "video" || i.kind === "image";

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="muted panel-note">{children}</p>;
}

export function TextPanel({ project, playhead, onEdit }: PanelProps) {
  const [q, setQ] = useState("");
  const track = firstTrack(project, "video", "t_text");
  const presets = TEXT_PRESETS.filter((p) => p.label.toLowerCase().includes(q.toLowerCase()));
  return (
    <>
      <div className="panel-search">
        <label className="search">
          <Icon name="search" size={15} />
          <input placeholder="Find a style…" value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
      </div>
      <h5 className="panel-sub">Titles — added at the playhead</h5>
      <div className="preset-grid">
        {presets.map((p) => (
          <button
            key={p.id}
            className="preset-card text-preset"
            data-testid={`text-preset-${p.id}`}
            disabled={!track}
            onClick={() =>
              track &&
              onEdit(
                addItems(
                  [{ trackId: track, type: "text", startTime: round(playhead), endTime: round(playhead + p.duration), name: p.label, ...p.fields }],
                  `add ${p.label.toLowerCase()}`,
                ),
              )
            }
          >
            <span className="preset-preview" style={p.style}>
              {p.sample}
            </span>
            <span className="preset-label">{p.label}</span>
          </button>
        ))}
      </div>
    </>
  );
}

const CAPTION_STYLES = [
  { id: "clean", label: "Clean", fields: { captionPresetId: "clean", captionTextColor: "#ffffff", captionHighlightColor: "#ffffff", captionFontSize: 54 }, css: { color: "#fff" } },
  { id: "pop", label: "Pop", fields: { captionPresetId: "pop", captionTextColor: "#ffffff", captionHighlightColor: "#ffd84d", captionFontSize: 64 }, css: { color: "#ffd84d", fontWeight: 800 } },
  { id: "boxed", label: "Boxed", fields: { captionPresetId: "boxed", captionTextColor: "#111111", captionHighlightColor: "#111111", captionFontSize: 52 }, css: { color: "#111", background: "#fff", padding: "2px 6px" } },
  { id: "neon", label: "Neon", fields: { captionPresetId: "neon", captionTextColor: "#c9f", captionHighlightColor: "#5ff", captionFontSize: 60 }, css: { color: "#5ff", textShadow: "0 0 6px #5ff" } },
];

export function CaptionsPanel({ project, playhead, selection, onEdit, onSelect }: PanelProps) {
  const track = firstTrack(project, "caption", "t_captions");
  const captions = project.items.filter((i) => i.kind === "caption");
  const target = selection.filter((i) => i.kind === "caption");
  return (
    <>
      <p className="muted panel-note">Automatic transcription arrives in a later version. Add a caption and type its lines in the properties.</p>
      <button
        className="btn wide"
        data-testid="caption-add"
        disabled={!track}
        onClick={() =>
          track &&
          onEdit(
            addItems(
              [
                {
                  trackId: track,
                  type: "caption",
                  startTime: round(playhead),
                  endTime: round(playhead + 3),
                  name: "Caption",
                  captionText: "Your words here",
                  captionSegments: [{ id: "s1", text: "Your words here", startTime: 0, endTime: 3 }],
                  ...CAPTION_STYLES[1].fields,
                },
              ],
              "add caption",
            ),
          )
        }
      >
        <Icon name="plus" size={15} /> Add caption at playhead
      </button>
      <h5 className="panel-sub">Styles {target.length ? `— apply to ${target.length} selected` : "— select a caption to apply"}</h5>
      <div className="preset-grid">
        {CAPTION_STYLES.map((s) => (
          <button
            key={s.id}
            className="preset-card"
            data-testid={`caption-style-${s.id}`}
            disabled={target.length === 0}
            onClick={() => onEdit(updateItems(target.map((t) => ({ id: t.id, set: s.fields })), `caption style ${s.label.toLowerCase()}`))}
          >
            <span className="preset-preview caption-sample" style={s.css}>
              Hello there
            </span>
            <span className="preset-label">{s.label}</span>
          </button>
        ))}
      </div>
      <h5 className="panel-sub">In this project</h5>
      {captions.length === 0 && <Empty>No captions yet.</Empty>}
      {captions.map((c) => (
        <button key={c.id} className="list-row" onClick={() => onSelect(c)}>
          <span className="muted small">{(c.start ?? 0).toFixed(1)}s</span> {c.text ?? c.name}
        </button>
      ))}
    </>
  );
}

export function TransitionsPanel({ project, selection, onEdit }: PanelProps) {
  const [duration, setDuration] = useState(0.5);
  const item = selection[selection.length - 1];
  const next = item ? nextAdjacent(project, item) : undefined;
  const apply = (type: string | null, where: "in" | "out" | "between") => {
    if (!item) return;
    const value = type ? { type, duration } : null;
    const set = (id: string, key: string) => (value ? updateItem(id, { [key]: value }, [], "transition") : updateItem(id, {}, [key], "remove transition"));
    if (where === "between" && next) {
      const edits = [
        value ? { id: item.id, set: { transitionOut: value } } : { id: item.id, unset: ["transitionOut"] },
        value ? { id: next.id, set: { transitionIn: value } } : { id: next.id, unset: ["transitionIn"] },
      ];
      onEdit(updateItems(edits, value ? "transition between clips" : "remove transition"));
    } else onEdit(set(item.id, where === "in" ? "transitionIn" : "transitionOut"));
  };
  return (
    <>
      {!item && <Empty>Select a clip on the timeline, then pick a transition.</Empty>}
      <label className="slider panel-slider">
        <span className="slider-label">Duration</span>
        <input type="range" min={0.1} max={3} step={0.1} value={duration} onChange={(e) => setDuration(Number(e.target.value))} />
        <span className="slider-value">{duration.toFixed(1)}s</span>
      </label>
      <div className="preset-grid">
        {TRANSITIONS.map((t) => (
          <div key={t.id} className="preset-card transition-card" data-testid={`transition-${t.id}`}>
            <span className={`preset-preview transition-swatch tr-${t.id}`} />
            <span className="preset-label">{t.label}</span>
            <div className="card-actions">
              <button className="chip-sm" disabled={!item} title="At the start of the selected clip" onClick={() => apply(t.type, "in")}>
                In
              </button>
              <button className="chip-sm" disabled={!item} title="At the end of the selected clip" onClick={() => apply(t.type, "out")}>
                Out
              </button>
              <button className="chip-sm" disabled={!next} title="Between the selected clip and the next one" data-testid={`transition-${t.id}-between`} onClick={() => apply(t.type, "between")}>
                Join
              </button>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

export function EffectsPanel({ project, playhead, selection, onEdit }: PanelProps) {
  const targets = selection.filter(visual);
  const track =
    project.tracks.find((t) => t.kind === "adjustment")?.id ?? firstTrack(project, "video", "t_overlay");
  const span = selection.length
    ? { start: Math.min(...selection.map((s) => s.start ?? 0)), end: Math.max(...selection.map((s) => s.end ?? 0)) }
    : { start: playhead, end: playhead + 5 };
  return (
    <>
      <p className="muted panel-note">
        Click a look to add it as an adjustment layer {selection.length ? "over the selected clips" : "at the playhead"}; use “On clip” to grade
        the selected clips directly.
      </p>
      <div className="preset-grid">
        {LOOK_PRESETS.map((l) => (
          <div key={l.id} className="preset-card" data-testid={`look-${l.id}`}>
            <button
              className="preset-preview look-swatch"
              style={{ background: l.swatch }}
              title="Add as adjustment layer"
              disabled={!track}
              data-testid={`look-${l.id}-layer`}
              onClick={() =>
                track &&
                onEdit(
                  addItems(
                    [{ trackId: track, type: "adjustment", startTime: round(span.start), endTime: round(span.end), name: l.label, adjustmentKind: "grade", colorGrading: l.grading }],
                    `add ${l.label.toLowerCase()} look`,
                  ),
                )
              }
            />
            <span className="preset-label">{l.label}</span>
            <div className="card-actions">
              <button
                className="chip-sm"
                disabled={targets.length === 0}
                data-testid={`look-${l.id}-clip`}
                onClick={() => onEdit(updateItems(targets.map((t) => ({ id: t.id, set: { colorGrading: l.grading } })), `${l.label.toLowerCase()} look`))}
              >
                On clip
              </button>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

export function AudioPanel({ project, selection, onEdit, onSelect }: PanelProps) {
  const sound = selection.filter((i) => i.kind === "video" || i.kind === "audio");
  const all = project.items.filter((i) => i.kind === "video" || i.kind === "audio");
  const music = project.items.filter((i) => i.kind === "audio");
  const fade = (key: "audioFadeIn" | "audioFadeOut", d: number) =>
    onEdit(updateItems(sound.map((s) => ({ id: s.id, set: { [key]: { duration: d } } })), key === "audioFadeIn" ? "fade in" : "fade out"));
  return (
    <>
      <h5 className="panel-sub">Selected {sound.length ? `(${sound.length})` : ""}</h5>
      {sound.length === 0 && <Empty>Select clips with sound to fade or mute them.</Empty>}
      <div className="chips-row">
        <button className="chip-sm" disabled={!sound.length} data-testid="audio-fade-in" onClick={() => fade("audioFadeIn", 1)}>
          Fade in 1s
        </button>
        <button className="chip-sm" disabled={!sound.length} data-testid="audio-fade-out" onClick={() => fade("audioFadeOut", 1)}>
          Fade out 1s
        </button>
        <button
          className="chip-sm"
          disabled={!sound.length}
          onClick={() => onEdit(updateItems(sound.map((s) => ({ id: s.id, unset: ["audioFadeIn", "audioFadeOut"] })), "no fades"))}
        >
          No fades
        </button>
        <button
          className="chip-sm"
          disabled={!sound.length}
          onClick={() => onEdit(updateItems(sound.map((s) => ({ id: s.id, set: { muted: !s.muted } })), "toggle mute"))}
        >
          Mute / unmute
        </button>
      </div>
      <h5 className="panel-sub">Mix</h5>
      <div className="chips-row">
        <button
          className="chip-sm"
          disabled={!all.length}
          data-testid="audio-normalize"
          onClick={() => onEdit(updateItems(all.map((s) => ({ id: s.id, set: { volume: 100 }, unset: ["muted"] })), "reset volumes"))}
        >
          Reset all to 100%
        </button>
        <button
          className="chip-sm"
          disabled={!music.length}
          data-testid="audio-duck"
          onClick={() => onEdit(updateItems(music.map((s) => ({ id: s.id, set: { volume: 30 } })), "duck music"))}
        >
          Music under dialogue (30%)
        </button>
      </div>
      <h5 className="panel-sub">Audio clips</h5>
      {music.length === 0 && <Empty>No audio clips. Import music or sound effects with Import.</Empty>}
      {music.map((m) => (
        <button key={m.id} className="list-row" onClick={() => onSelect(m)}>
          <Icon name="audio" size={13} /> {m.name}
          <span className="muted small"> {((m.end ?? 0) - (m.start ?? 0)).toFixed(1)}s</span>
        </button>
      ))}
    </>
  );
}

const PLACEMENTS = [
  { id: "fill", label: "Fill frame", set: { autoReframe: true }, unset: ["transform"] },
  { id: "fit", label: "Fit inside", set: { autoReframe: false }, unset: ["transform"] },
  { id: "pip-br", label: "Corner (bottom right)", set: { autoReframe: false, transform: { x: 560, y: 300, scale: 0.3 } }, unset: [] },
  { id: "pip-tl", label: "Corner (top left)", set: { autoReframe: false, transform: { x: -560, y: -300, scale: 0.3 } }, unset: [] },
  { id: "left-half", label: "Left half", set: { autoReframe: false, transform: { x: -480, scale: 0.5 } }, unset: [] },
  { id: "right-half", label: "Right half", set: { autoReframe: false, transform: { x: 480, scale: 0.5 } }, unset: [] },
];

export function LayoutPanel({ project, selection, onEdit }: PanelProps) {
  const targets = selection.filter(visual);
  return (
    <>
      <h5 className="panel-sub">Frame</h5>
      <div className="aspect-grid">
        {ASPECTS.map((a) => {
          const [w, h] = a.id.split(":").map(Number);
          return (
            <button
              key={a.id}
              className={`aspect-card${project.aspectRatio === a.id ? " on" : ""}`}
              data-testid={`aspect-${a.id.replace(":", "x")}`}
              onClick={() => onEdit(commands.setAspectRatio(a.id))}
            >
              <span className="aspect-shape" style={{ aspectRatio: `${w} / ${h}` }} />
              <strong>{a.id}</strong>
              <span className="muted small">{a.label}</span>
            </button>
          );
        })}
      </div>
      <h5 className="panel-sub">Placement {targets.length ? `— ${targets.length} selected` : ""}</h5>
      {targets.length === 0 && <Empty>Select a video or image clip to place it in the frame.</Empty>}
      <div className="chips-row">
        {PLACEMENTS.map((p) => (
          <button
            key={p.id}
            className="chip-sm"
            disabled={!targets.length}
            data-testid={`place-${p.id}`}
            onClick={() => onEdit(updateItems(targets.map((t) => ({ id: t.id, set: p.set, unset: p.unset })), p.label.toLowerCase()))}
          >
            {p.label}
          </button>
        ))}
      </div>
    </>
  );
}
