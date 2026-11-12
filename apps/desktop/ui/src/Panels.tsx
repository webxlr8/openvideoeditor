// Left-rail tool panels: Text, Transitions, Effects, Audio, Layout (Captions: CaptionsPanel.tsx). Each action is one
// revision.

import { useEffect, useState } from "react";
import type {
  AppError,
  ItemView_Serialize,
  ProjectView_Serialize,
} from "./bindings";
import { commands } from "./bridge";
import {
  ASPECTS,
  type EditResult,
  type Fields,
  LOOK_PRESETS,
  TEXT_PRESETS,
  addItems,
  firstTrack,
  itemDetail,
  nextAdjacent,
  updateItem,
  updateItems,
  snapToFrame,
} from "./edit";
import {
  type EffectSpec,
  GROUP_LABEL,
  type TransitionSpec,
  effectStack,
  newEffect,
  splitTransition,
  useCatalog,
  usePreview,
} from "./fx";
import { Icon } from "./icons";
import { fire } from "./notify";
import { TextAnimationPicker } from "./Motion";

export interface PanelProps {
  project: ProjectView_Serialize;
  /** Timeline time of the playhead, seconds. */
  playhead: number;
  /** Selected items, primary last. */
  selection: ItemView_Serialize[];
  onEdit: (p: EditResult) => void;
  onSelect: (item: ItemView_Serialize) => void;
  /** Shows an error toast. */
  onError?: (e: AppError) => void;
}

const round = (t: number) => snapToFrame(t);
const visual = (i: ItemView_Serialize) =>
  i.kind === "video" || i.kind === "image";

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="muted panel-note">{children}</p>;
}

export function TextPanel({
  project,
  playhead,
  selection,
  onEdit,
}: PanelProps) {
  const [q, setQ] = useState("");
  const track = firstTrack(project, "video", "t_text");
  const presets = TEXT_PRESETS.filter((p) =>
    p.label.toLowerCase().includes(q.toLowerCase()),
  );
  const texts = selection.filter((i) => i.kind === "text");
  const primaryId = texts[texts.length - 1]?.id;
  const [animation, setAnimation] = useState<unknown>(undefined);
  useEffect(() => {
    let live = true;
    if (primaryId)
      fire(
        itemDetail(primaryId).then((d) => {
          if (live) setAnimation(d?.textAnimation);
        }),
      );
    return () => {
      live = false;
    };
  }, [primaryId, project.rev]);
  return (
    <>
      <h5 className="panel-sub">
        Animation {texts.length ? `— ${texts.length} selected` : ""}
      </h5>
      {texts.length === 0 ? (
        <Empty>Select a title on the timeline to animate it in and out.</Empty>
      ) : (
        <TextAnimationPicker
          targets={texts.map((t) => ({ id: t.id, textAnimation: animation }))}
          onEdit={onEdit}
        />
      )}
      <div className="panel-search">
        <label className="search">
          <Icon name="search" size={15} />
          <input
            placeholder="Find a style…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
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
                  [
                    {
                      trackId: track,
                      type: "text",
                      startTime: round(playhead),
                      endTime: round(playhead + p.duration),
                      name: p.label,
                      ...p.fields,
                    },
                  ],
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

export function TransitionsPanel({ project, selection, onEdit }: PanelProps) {
  const catalog = useCatalog();
  const [duration, setDuration] = useState(0.5);
  const [easing, setEasing] = useState("ease-in-out");
  const [direction, setDirection] = useState("left");
  const item = selection[selection.length - 1];
  const next = item ? nextAdjacent(project, item) : undefined;
  const apply = (t: TransitionSpec | null, where: "in" | "out" | "between") => {
    if (!item) return;
    const value = t
      ? {
          type: t.id,
          duration,
          easing,
          ...(t.directional ? { direction } : {}),
        }
      : null;
    const set = (id: string, key: string) =>
      value
        ? updateItem(id, { [key]: value }, [], "transition")
        : updateItem(id, {}, [key], "remove transition");
    if (where === "between" && next) {
      const edits = [
        { id: item.id, unset: ["transitionOut"] },
        value
          ? { id: next.id, set: { transitionIn: value } }
          : { id: next.id, unset: ["transitionIn"] },
      ];
      onEdit(
        updateItems(
          edits,
          value ? "transition between clips" : "remove transition",
        ),
      );
    } else
      onEdit(set(item.id, where === "in" ? "transitionIn" : "transitionOut"));
  };
  const actions = (t: TransitionSpec | null) => (
    <div className="card-actions">
      <button
        className="chip-sm"
        disabled={!item}
        title="At the start of the selected clip"
        onClick={() => apply(t, "in")}
      >
        In
      </button>
      <button
        className="chip-sm"
        disabled={!item}
        title="At the end of the selected clip"
        onClick={() => apply(t, "out")}
      >
        Out
      </button>
      <button
        className="chip-sm"
        disabled={!next}
        title="Between the selected clip and the next one"
        data-testid={`transition-${t?.id ?? "none"}-between`}
        onClick={() => apply(t, "between")}
      >
        Join
      </button>
    </div>
  );
  return (
    <>
      {!item && (
        <Empty>Select a clip on the timeline, then pick a transition.</Empty>
      )}
      <label className="slider panel-slider">
        <span className="slider-label">Duration</span>
        <input
          type="range"
          min={0.1}
          max={3}
          step={0.1}
          value={duration}
          data-testid="transition-duration"
          onChange={(e) => setDuration(Number(e.target.value))}
        />
        <span className="slider-value">{duration.toFixed(1)}s</span>
      </label>
      <label className="select-row">
        <span>Easing</span>
        <select
          value={easing}
          data-testid="transition-easing"
          onChange={(e) => setEasing(e.target.value)}
        >
          {(
            catalog?.easings ?? ["linear", "ease-in", "ease-out", "ease-in-out"]
          ).map((x) => (
            <option key={x}>{x}</option>
          ))}
        </select>
      </label>
      <div
        className="chips-row"
        role="group"
        aria-label="Direction (wipe, push, fly)"
      >
        <span className="muted small">Direction</span>
        {(catalog?.directions ?? []).map((d) => (
          <button
            key={d}
            className={`chip-sm${direction === d ? " on" : ""}`}
            data-testid={`transition-dir-${d}`}
            onClick={() => setDirection(d)}
          >
            {DIR_ARROW[d] ?? ""} {d}
          </button>
        ))}
      </div>
      {!catalog && <Empty>Loading transitions…</Empty>}
      <div className="preset-grid">
        <div
          className="preset-card transition-card"
          data-testid="transition-none"
        >
          <span className="preset-preview transition-swatch tr-none" />
          <span className="preset-label">None</span>
          {actions(null)}
        </div>
        {catalog?.transitions.map((t) => (
          <div
            key={t.id}
            className="preset-card transition-card"
            data-testid={`transition-${t.id}`}
          >
            <span
              className={`preset-preview transition-swatch tr-${splitTransition(t.id).base}${t.directional || /-(left|right|up|down)$/.test(t.id) ? ` dir-${splitTransition(t.id, direction).direction}` : ""}`}
            />
            <span className="preset-label">
              {t.label}
              {t.audioOnly && <em className="muted small"> (sound)</em>}
            </span>
            {actions(t)}
          </div>
        ))}
      </div>
    </>
  );
}

const DIR_ARROW: Record<string, string> = {
  left: "←",
  right: "→",
  up: "↑",
  down: "↓",
};

function FxCard({
  spec,
  onClip,
  onLayer,
  canClip,
  canLayer,
}: {
  spec: EffectSpec;
  onClip: () => void;
  onLayer: () => void;
  canClip: boolean;
  canLayer: boolean;
}) {
  const url = usePreview("effect", spec.id);
  return (
    <div className="preset-card fx-card" data-testid={`effect-${spec.id}`}>
      <button
        className="preset-preview fx-preview"
        title={
          spec.group === "background"
            ? "Add as a background layer"
            : "Add as an effect layer"
        }
        disabled={!canLayer}
        data-testid={`effect-${spec.id}-layer`}
        onClick={onLayer}
      >
        {url ? <img src={url} alt="" /> : null}
      </button>
      <span className="preset-label">{spec.label}</span>
      <div className="card-actions">
        <button
          className="chip-sm"
          disabled={!canClip}
          data-testid={`effect-${spec.id}-clip`}
          onClick={onClip}
        >
          On clip
        </button>
      </div>
    </div>
  );
}

function LutCard({
  id,
  label,
  onClip,
  onLayer,
  canClip,
  canLayer,
}: {
  id: string;
  label: string;
  onClip: () => void;
  onLayer: () => void;
  canClip: boolean;
  canLayer: boolean;
}) {
  const url = usePreview("lut", id);
  return (
    <div className="preset-card" data-testid={`lut-look-${id}`}>
      <button
        className="preset-preview fx-preview"
        title="Add as adjustment layer"
        disabled={!canLayer}
        data-testid={`lut-look-${id}-layer`}
        onClick={onLayer}
      >
        {url ? <img src={url} alt="" /> : null}
      </button>
      <span className="preset-label">{label}</span>
      <div className="card-actions">
        <button
          className="chip-sm"
          disabled={!canClip}
          data-testid={`lut-look-${id}-clip`}
          onClick={onClip}
        >
          On clip
        </button>
      </div>
    </div>
  );
}

/** Edits each target's current fields (fetched fresh, so stacks and grades are extended, not replaced). */
async function mergeEach(
  ids: string[],
  f: (d: Fields) => Fields,
  summary: string,
) {
  const details = await Promise.all(ids.map((id) => itemDetail(id)));
  return updateItems(
    ids.map((id, i) => ({ id, set: f(details[i] ?? {}) })),
    summary,
  );
}

export function EffectsPanel({
  project,
  playhead,
  selection,
  onEdit,
}: PanelProps) {
  const catalog = useCatalog();
  const [query, setQuery] = useState("");
  const targets = selection.filter(visual);
  const track =
    project.tracks.find((t) => t.kind === "adjustment")?.id ??
    firstTrack(project, "video", "t_overlay");
  const span = selection.length
    ? {
        start: Math.min(...selection.map((s) => s.start ?? 0)),
        end: Math.max(...selection.map((s) => s.end ?? 0)),
      }
    : { start: playhead, end: playhead + 5 };
  const ids = targets.map((t) => t.id);
  const layer = (fields: Fields, summary: string) =>
    track &&
    onEdit(
      addItems(
        [
          {
            trackId: track,
            startTime: round(span.start),
            endTime: round(span.end),
            ...fields,
          },
        ],
        summary,
      ),
    );
  const q = query.trim().toLowerCase();
  const effects = (catalog?.effects ?? []).filter(
    (e) => !q || e.label.toLowerCase().includes(q) || e.id.includes(q),
  );
  return (
    <>
      <p className="muted panel-note">
        Click a preview to add it as a layer{" "}
        {selection.length ? "over the selected clips" : "at the playhead"}; “On
        clip” applies it to the selected clips directly.
      </p>
      <input
        className="search-field"
        placeholder="Search effects"
        value={query}
        data-testid="effects-search"
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => e.stopPropagation()}
      />
      {!q && (
        <>
          <h5 className="panel-sub">Looks</h5>
          <div className="preset-grid">
            {LOOK_PRESETS.map((l) => (
              <div
                key={l.id}
                className="preset-card"
                data-testid={`look-${l.id}`}
              >
                <button
                  className="preset-preview look-swatch"
                  style={{ background: l.swatch }}
                  title="Add as adjustment layer"
                  disabled={!track}
                  data-testid={`look-${l.id}-layer`}
                  onClick={() =>
                    layer(
                      {
                        type: "adjustment",
                        name: l.label,
                        adjustmentKind: "grade",
                        colorGrading: l.grading,
                      },
                      `add ${l.label.toLowerCase()} look`,
                    )
                  }
                />
                <span className="preset-label">{l.label}</span>
                <div className="card-actions">
                  <button
                    className="chip-sm"
                    disabled={targets.length === 0}
                    data-testid={`look-${l.id}-clip`}
                    onClick={() =>
                      onEdit(
                        updateItems(
                          targets.map((t) => ({
                            id: t.id,
                            set: { colorGrading: l.grading },
                          })),
                          `${l.label.toLowerCase()} look`,
                        ),
                      )
                    }
                  >
                    On clip
                  </button>
                </div>
              </div>
            ))}
            {catalog?.luts.map((l) => (
              <LutCard
                key={l.id}
                id={l.id}
                label={`${l.label} LUT`}
                canClip={targets.length > 0}
                canLayer={!!track}
                onLayer={() =>
                  layer(
                    {
                      type: "adjustment",
                      name: `${l.label} look`,
                      adjustmentKind: "lut",
                      lutPresetId: l.id,
                      lutIntensity: 1,
                    },
                    `add ${l.label.toLowerCase()} LUT`,
                  )
                }
                onClip={() =>
                  onEdit(
                    mergeEach(
                      ids,
                      (d) => {
                        const g = (
                          d.colorGrade && typeof d.colorGrade === "object"
                            ? d.colorGrade
                            : {}
                        ) as Fields;
                        return {
                          colorGrade: {
                            ...g,
                            lut: { intensity: 1, stage: "look", preset: l.id },
                          },
                        };
                      },
                      `${l.label.toLowerCase()} LUT`,
                    ),
                  )
                }
              />
            ))}
          </div>
        </>
      )}
      {!catalog && <Empty>Loading effects…</Empty>}
      {(["stylize", "distort", "background"] as const).map((g) => {
        const list = effects.filter((e) => e.group === g);
        if (!list.length) return null;
        return (
          <div key={g}>
            <h5 className="panel-sub">{GROUP_LABEL[g]}</h5>
            <div className="preset-grid">
              {list.map((e) => (
                <FxCard
                  key={e.id}
                  spec={e}
                  canClip={targets.length > 0}
                  canLayer={!!track}
                  onLayer={() =>
                    layer(
                      {
                        type: "shader",
                        name: e.label,
                        shaderComposition: {
                          template: e.id,
                          params: {},
                          amount: 100,
                          region: "all",
                        },
                      },
                      `add ${e.label.toLowerCase()} layer`,
                    )
                  }
                  onClip={() =>
                    onEdit(
                      mergeEach(
                        ids,
                        (d) => ({ effects: [...effectStack(d), newEffect(e)] }),
                        `add ${e.label.toLowerCase()}`,
                      ),
                    )
                  }
                />
              ))}
            </div>
          </div>
        );
      })}
    </>
  );
}

export function AudioPanel({
  project,
  selection,
  onEdit,
  onSelect,
}: PanelProps) {
  const sound = selection.filter(
    (i) => i.kind === "video" || i.kind === "audio",
  );
  const all = project.items.filter(
    (i) => i.kind === "video" || i.kind === "audio",
  );
  const music = project.items.filter((i) => i.kind === "audio");
  const fade = (key: "audioFadeIn" | "audioFadeOut", d: number) =>
    onEdit(
      updateItems(
        sound.map((s) => ({ id: s.id, set: { [key]: { duration: d } } })),
        key === "audioFadeIn" ? "fade in" : "fade out",
      ),
    );
  return (
    <>
      <h5 className="panel-sub">
        Selected {sound.length ? `(${sound.length})` : ""}
      </h5>
      {sound.length === 0 && (
        <Empty>Select clips with sound to fade or mute them.</Empty>
      )}
      <div className="chips-row">
        <button
          className="chip-sm"
          disabled={!sound.length}
          data-testid="audio-fade-in"
          onClick={() => fade("audioFadeIn", 1)}
        >
          Fade in 1s
        </button>
        <button
          className="chip-sm"
          disabled={!sound.length}
          data-testid="audio-fade-out"
          onClick={() => fade("audioFadeOut", 1)}
        >
          Fade out 1s
        </button>
        <button
          className="chip-sm"
          disabled={!sound.length}
          onClick={() =>
            onEdit(
              updateItems(
                sound.map((s) => ({
                  id: s.id,
                  unset: ["audioFadeIn", "audioFadeOut"],
                })),
                "no fades",
              ),
            )
          }
        >
          No fades
        </button>
        <button
          className="chip-sm"
          disabled={!sound.length}
          onClick={() =>
            onEdit(
              updateItems(
                sound.map((s) => ({ id: s.id, set: { muted: !s.muted } })),
                "toggle mute",
              ),
            )
          }
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
          title="Sets every clip's volume to 100 % and unmutes it. It does not measure or even out loudness."
          onClick={() =>
            onEdit(
              updateItems(
                all.map((s) => ({
                  id: s.id,
                  set: { volume: 100 },
                  unset: ["muted"],
                })),
                "reset volumes",
              ),
            )
          }
        >
          Reset volumes to 100 %
        </button>
        <button
          className="chip-sm"
          disabled={!music.length}
          data-testid="audio-duck"
          title="Sets every audio clip (music and sound effects) to 30 % for its whole length. It does not follow the dialogue."
          onClick={() =>
            onEdit(
              updateItems(
                music.map((s) => ({ id: s.id, set: { volume: 30 } })),
                "lower music to 30%",
              ),
            )
          }
        >
          Lower music to 30 %
        </button>
      </div>
      <h5 className="panel-sub">Audio clips</h5>
      {music.length === 0 && (
        <Empty>
          No audio clips. Import music or sound effects with Import.
        </Empty>
      )}
      {music.map((m) => (
        <button key={m.id} className="list-row" onClick={() => onSelect(m)}>
          <Icon name="audio" size={13} /> {m.name}
          <span className="muted small">
            {" "}
            {((m.end ?? 0) - (m.start ?? 0)).toFixed(1)}s
          </span>
        </button>
      ))}
    </>
  );
}

const PLACEMENTS = [
  {
    id: "fill",
    label: "Fill frame",
    set: { autoReframe: true },
    unset: ["transform"],
  },
  {
    id: "fit",
    label: "Fit inside",
    set: { autoReframe: false },
    unset: ["transform"],
  },
  {
    id: "pip-br",
    label: "Corner (bottom right)",
    set: { autoReframe: false, transform: { x: 560, y: 300, scale: 0.3 } },
    unset: [],
  },
  {
    id: "pip-tl",
    label: "Corner (top left)",
    set: { autoReframe: false, transform: { x: -560, y: -300, scale: 0.3 } },
    unset: [],
  },
  {
    id: "left-half",
    label: "Left half",
    set: { autoReframe: false, transform: { x: -480, scale: 0.5 } },
    unset: [],
  },
  {
    id: "right-half",
    label: "Right half",
    set: { autoReframe: false, transform: { x: 480, scale: 0.5 } },
    unset: [],
  },
];

export function LayoutPanel({ project, selection, onEdit }: PanelProps) {
  const catalog = useCatalog();
  const targets = selection.filter((i) => visual(i) || i.kind === "text");
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
              <span
                className="aspect-shape"
                style={{ aspectRatio: `${w} / ${h}` }}
              />
              <strong>{a.id}</strong>
              <span className="muted small">{a.label}</span>
            </button>
          );
        })}
      </div>
      <h5 className="panel-sub">
        Layouts {targets.length ? `— ${targets.length} selected` : ""}
      </h5>
      <p className="muted panel-note">
        Select clips (shift-click for several), then pick a layout: they fill
        its slots in timeline order. Clips on a Layout track arrange themselves.
      </p>
      <div className="layout-grid">
        {(catalog?.layouts ?? []).map((l) => (
          <button
            key={l.id}
            className="layout-card"
            title={l.label}
            disabled={!targets.length}
            data-testid={`layout-${l.id}`}
            onClick={() => {
              const ordered = [...targets].sort(
                (a, b) =>
                  (a.start ?? 0) - (b.start ?? 0) ||
                  a.trackId.localeCompare(b.trackId),
              );
              onEdit(
                updateItems(
                  ordered.map((t, i) => ({
                    id: t.id,
                    set: { layout: { preset: l.id, slot: i % l.slots.length } },
                    unset: ["transform"],
                  })),
                  `${l.label.toLowerCase()} layout`,
                ),
              );
            }}
          >
            <svg viewBox="0 0 160 90" className="layout-shape">
              {l.slots.map((s, i) => (
                <rect
                  key={i}
                  x={s[0] * 160 + 2}
                  y={s[1] * 90 + 2}
                  width={Math.max(1, s[2] * 160 - 4)}
                  height={Math.max(1, s[3] * 90 - 4)}
                  rx={3}
                />
              ))}
            </svg>
            <span className="small">{l.label}</span>
          </button>
        ))}
      </div>
      <div className="chips-row">
        <button
          className="chip-sm"
          disabled={!targets.length}
          data-testid="layout-none"
          onClick={() =>
            onEdit(
              updateItems(
                targets.map((t) => ({ id: t.id, unset: ["layout"] })),
                "leave layout",
              ),
            )
          }
        >
          Remove from layout
        </button>
      </div>
      <h5 className="panel-sub">
        Placement {targets.length ? `— ${targets.length} selected` : ""}
      </h5>
      {targets.length === 0 && (
        <Empty>Select a video or image clip to place it in the frame.</Empty>
      )}
      <div className="chips-row">
        {PLACEMENTS.map((p) => (
          <button
            key={p.id}
            className="chip-sm"
            disabled={!targets.length}
            data-testid={`place-${p.id}`}
            onClick={() =>
              onEdit(
                updateItems(
                  targets.map((t) => ({
                    id: t.id,
                    set: p.set,
                    unset: [...p.unset, "layout"],
                  })),
                  p.label.toLowerCase(),
                ),
              )
            }
          >
            {p.label}
          </button>
        ))}
      </div>
    </>
  );
}
