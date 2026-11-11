// Properties of the selected item. Every control commits one revision when the gesture ends (slider released,
// field confirmed); while dragging, the value is shown locally.

import { useEffect, useState } from "react";
import type { ItemView_Serialize, ProjectView_Serialize } from "./bindings";
import { commands } from "./bridge";
import {
  CaptionSample,
  type CaptionSegment,
  DEFAULT_CAPTION_PRESET,
  animationLabel,
  retimeSegment,
  useCaptionPresets,
} from "./CaptionsPanelStyles";
import {
  type EditResult,
  type Fields,
  TRANSITIONS,
  itemDetail,
  updateItem,
} from "./edit";
import { useCatalog } from "./fx";
import {
  ColorGradeEditor,
  EffectStackEditor,
  ShaderEditor,
  TransitionEditor,
} from "./FxControls";
import { Icon } from "./icons";
import { CodeTab, FontPicker } from "./Motion";

type Tab =
  | "general"
  | "color"
  | "effects"
  | "shader"
  | "audio"
  | "text"
  | "caption"
  | "code";

const TABS: Record<string, Tab[]> = {
  video: ["general", "color", "effects", "audio"],
  image: ["general", "color", "effects"],
  audio: ["audio", "general"],
  text: ["text", "general"],
  caption: ["caption", "general"],
  adjustment: ["color", "effects", "general"],
  code: ["code", "general"],
  shader: ["shader", "general"],
};
const TAB_LABEL: Record<Tab, string> = {
  general: "General",
  color: "Color",
  effects: "Effects",
  shader: "Shader",
  audio: "Audio",
  text: "Text",
  caption: "Caption",
  code: "Code",
};

const num = (v: unknown, d: number) =>
  typeof v === "number" && Number.isFinite(v) ? v : d;
const str = (v: unknown, d: string) => (typeof v === "string" ? v : d);
const obj = (v: unknown): Fields =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Fields) : {};

/** A slider that previews locally and commits once on release. */
export function Slider({
  label,
  value,
  min,
  max,
  step = 1,
  format = (v) => String(Math.round(v * 100) / 100),
  onCommit,
  testId,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  onCommit: (v: number) => void;
  testId?: string;
}) {
  const [live, setLive] = useState<number | null>(null);
  const shown = live ?? value;
  const commit = () => {
    if (live !== null && live !== value) onCommit(live);
    setLive(null);
  };
  return (
    <label className="slider">
      <span className="slider-label">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={shown}
        data-testid={testId}
        onChange={(e) => setLive(Number(e.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
      />
      <span className="slider-value">{format(shown)}</span>
    </label>
  );
}

/** A number field that commits on Enter or blur. */
export function NumberField({
  label,
  value,
  unit,
  step = 1,
  onCommit,
  testId,
}: {
  label: string;
  value: number;
  unit?: string;
  step?: number;
  onCommit: (v: number) => void;
  testId?: string;
}) {
  const [text, setText] = useState<string | null>(null);
  const commit = () => {
    if (text !== null) {
      const v = Number(text);
      if (Number.isFinite(v) && v !== value) onCommit(v);
    }
    setText(null);
  };
  return (
    <label className="num-field">
      <span>{label}</span>
      <input
        inputMode="decimal"
        value={text ?? String(Math.round(value * 1000) / 1000)}
        step={step}
        data-testid={testId}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") {
            setText(null);
            (e.target as HTMLInputElement).blur();
          }
          e.stopPropagation();
        }}
      />
      {unit && <em>{unit}</em>}
    </label>
  );
}

function TextArea({
  value,
  onCommit,
  testId,
  rows = 3,
}: {
  value: string;
  onCommit: (v: string) => void;
  testId?: string;
  rows?: number;
}) {
  const [text, setText] = useState<string | null>(null);
  return (
    <textarea
      className="field-text"
      rows={rows}
      value={text ?? value}
      data-testid={testId}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        if (text !== null && text !== value) onCommit(text);
        setText(null);
      }}
      onKeyDown={(e) => e.stopPropagation()}
    />
  );
}

function Toggle({
  label,
  on,
  onChange,
  testId,
}: {
  label: string;
  on: boolean;
  onChange: (v: boolean) => void;
  testId?: string;
}) {
  return (
    <button
      className={`toggle-row${on ? " on" : ""}`}
      role="switch"
      aria-checked={on}
      data-testid={testId}
      onClick={() => onChange(!on)}
    >
      <span>{label}</span>
      <span className="switch" />
    </button>
  );
}

function Section({
  title,
  children,
  aside,
}: {
  title: string;
  children: React.ReactNode;
  aside?: React.ReactNode;
}) {
  return (
    <section className="insp-section">
      <header>
        <h4>{title}</h4>
        {aside}
      </header>
      {children}
    </section>
  );
}

function ColorField({
  label,
  value,
  onCommit,
}: {
  label: string;
  value: string;
  onCommit: (v: string) => void;
}) {
  const hex = /^#[0-9a-f]{6}/i.test(value) ? value.slice(0, 7) : "#ffffff";
  return (
    <label className="color-field">
      <span>{label}</span>
      <input
        type="color"
        value={hex}
        onChange={(e) => onCommit(e.target.value)}
      />
      <code>{hex}</code>
    </label>
  );
}

const SPEEDS = [0.5, 1, 1.25, 1.5, 2, 4];
const BLENDS = [
  "normal",
  "multiply",
  "screen",
  "overlay",
  "darken",
  "lighten",
  "soft-light",
  "hard-light",
  "difference",
];

export function Inspector({
  project,
  item,
  onEdit,
  onClose,
  onCollapse,
  playhead = 0,
}: {
  project: ProjectView_Serialize;
  item: ItemView_Serialize;
  onEdit: (p: EditResult) => void;
  onClose: () => void;
  onCollapse: () => void;
  /** Timeline time of the playhead, seconds (colour match reference, scopes). */
  playhead?: number;
}) {
  const [detail, setDetail] = useState<Fields | null>(null);
  const catalog = useCatalog();
  const tabs = TABS[item.kind] ?? ["general"];
  const [tabPick, setTab] = useState<Tab | null>(null);
  const tab = tabPick && tabs.includes(tabPick) ? tabPick : tabs[0];

  useEffect(() => {
    let live = true;
    void itemDetail(item.id).then((d) => live && setDetail(d));
    return () => {
      live = false;
    };
  }, [item.id, project.rev]);

  const d = detail ?? {};
  const set = (fields: Fields, summary?: string) =>
    onEdit(updateItem(item.id, fields, [], summary ?? null));
  const unset = (keys: string[], summary?: string) =>
    onEdit(updateItem(item.id, {}, keys, summary ?? null));
  const setIn = (key: string, sub: string, v: unknown, summary?: string) =>
    set({ [key]: { ...obj(d[key]), [sub]: v } }, summary);
  const track = project.tracks.find((t) => t.id === item.trackId);
  const duration = (item.end ?? 0) - (item.start ?? 0);
  const transform = obj(d.transform);
  const crop = obj(d.crop);
  const grading = obj(d.colorGrading);

  const transitionRow = (
    key: "transitionIn" | "transitionOut",
    label: string,
  ) => {
    const t = obj(d[key]);
    const type = str(t.type, "none");
    return (
      <div className="transition-row">
        <span>{label}</span>
        <select
          value={TRANSITIONS.some((x) => x.type === type) ? type : "none"}
          data-testid={`insp-${key}`}
          onChange={(e) =>
            e.target.value === "none"
              ? unset([key], `remove ${label.toLowerCase()} transition`)
              : set(
                  {
                    [key]: {
                      ...t,
                      type: e.target.value,
                      duration: num(t.duration, 0.5),
                    },
                  },
                  `${label.toLowerCase()} transition`,
                )
          }
        >
          {TRANSITIONS.map((x) => (
            <option key={x.id} value={x.type ?? "none"}>
              {x.label}
            </option>
          ))}
        </select>
        {d[key] !== undefined && (
          <NumberField
            label=""
            unit="s"
            value={num(t.duration, 0.5)}
            onCommit={(v) =>
              set({ [key]: { ...t, duration: Math.max(0.05, v) } })
            }
          />
        )}
      </div>
    );
  };

  return (
    <aside className="side-panel inspector" data-testid="inspector">
      <header className="insp-head">
        <div>
          <div className="crumbs">
            <span className="muted">{track?.name ?? "Track"}</span> /{" "}
            <strong>{item.name}</strong>
          </div>
          <div className="muted small">
            {item.kind[0].toUpperCase() + item.kind.slice(1)} ·{" "}
            {duration.toFixed(1)}s
          </div>
        </div>
        <span className="insp-head-tools">
          <button
            className="icon-btn"
            title="Collapse panel"
            onClick={onCollapse}
            data-testid="collapse-panel"
          >
            <Icon name="panel" />
          </button>
          <button
            className="icon-btn"
            title="Close properties"
            onClick={onClose}
            data-testid="close-inspector"
          >
            <Icon name="close" size={16} />
          </button>
        </span>
      </header>
      {tabs.length > 1 && (
        <div className="tabs insp-tabs" role="tablist">
          {tabs.map((t) => (
            <button
              key={t}
              role="tab"
              className={t === tab ? "on" : ""}
              aria-selected={t === tab}
              data-testid={`insp-tab-${t}`}
              onClick={() => setTab(t)}
            >
              {TAB_LABEL[t]}
            </button>
          ))}
        </div>
      )}
      <div className="quick-actions">
        <button
          className={`icon-btn${item.hidden ? " on" : ""}`}
          title={item.hidden ? "Show" : "Hide"}
          data-testid="insp-hide"
          onClick={() =>
            set({ hidden: !item.hidden }, item.hidden ? "show" : "hide")
          }
        >
          <Icon name="eye" size={16} />
        </button>
        <button
          className="icon-btn"
          title="Duplicate (D)"
          data-testid="insp-duplicate"
          onClick={() => onEdit(commands.duplicateItems([item.id]))}
        >
          <Icon name="copy" size={16} />
        </button>
        {(item.kind === "video" || item.kind === "audio") && (
          <button
            className={`icon-btn${item.muted ? " on" : ""}`}
            title={item.muted ? "Unmute" : "Mute"}
            data-testid="insp-mute"
            onClick={() =>
              set({ muted: !item.muted }, item.muted ? "unmute" : "mute")
            }
          >
            <Icon name={item.muted ? "mute" : "volume"} size={16} />
          </button>
        )}
        <button
          className="icon-btn danger"
          title="Delete"
          data-testid="insp-delete"
          onClick={() => onEdit(commands.removeItems([item.id]))}
        >
          <Icon name="close" size={16} />
        </button>
      </div>
      <div className="insp-body">
        {detail === null && <p className="muted">Loading…</p>}
        {detail !== null && tab === "general" && (
          <>
            {(item.kind === "video" || item.kind === "audio") && (
              <Section title="Playback">
                {item.kind === "video" && (
                  <Toggle
                    label="Reverse"
                    on={d.playbackDirection === "reverse"}
                    testId="insp-reverse"
                    onChange={(v) =>
                      v
                        ? set({ playbackDirection: "reverse" }, "reverse")
                        : unset(["playbackDirection"], "forward")
                    }
                  />
                )}
                <Slider
                  label="Speed"
                  value={num(d.playbackSpeed, 1)}
                  min={0.25}
                  max={4}
                  step={0.05}
                  format={(v) => `${v}×`}
                  testId="insp-speed"
                  onCommit={(v) => set({ playbackSpeed: v }, "speed")}
                />
                <div className="chips-row">
                  {SPEEDS.map((s) => (
                    <button
                      key={s}
                      className={`chip-sm${num(d.playbackSpeed, 1) === s ? " on" : ""}`}
                      onClick={() => set({ playbackSpeed: s }, "speed")}
                    >
                      {s}×
                    </button>
                  ))}
                </div>
              </Section>
            )}
            {item.kind !== "audio" && (
              <>
                <Section
                  title="Transform"
                  aside={
                    <button
                      className="link"
                      onClick={() => unset(["transform"], "reset transform")}
                    >
                      Reset
                    </button>
                  }
                >
                  <div className="grid2">
                    <NumberField
                      label="X"
                      unit="px"
                      value={num(transform.x, 0)}
                      testId="insp-x"
                      onCommit={(v) => setIn("transform", "x", v, "move")}
                    />
                    <NumberField
                      label="Y"
                      unit="px"
                      value={num(transform.y, 0)}
                      testId="insp-y"
                      onCommit={(v) => setIn("transform", "y", v, "move")}
                    />
                    <NumberField
                      label="Scale"
                      value={num(transform.scale, 1)}
                      step={0.05}
                      testId="insp-scale"
                      onCommit={(v) =>
                        setIn("transform", "scale", Math.max(0, v), "scale")
                      }
                    />
                    <NumberField
                      label="Rotation"
                      unit="°"
                      value={num(transform.rotation, 0)}
                      testId="insp-rotation"
                      onCommit={(v) =>
                        setIn("transform", "rotation", v, "rotate")
                      }
                    />
                  </div>
                  <Toggle
                    label="Flip horizontally"
                    on={transform.flipX === true}
                    onChange={(v) => setIn("transform", "flipX", v, "flip")}
                  />
                </Section>
                <Section title="Look">
                  <Slider
                    label="Opacity"
                    value={num(d.opacity, 1)}
                    min={0}
                    max={1}
                    step={0.01}
                    format={(v) => `${Math.round(v * 100)}%`}
                    testId="insp-opacity"
                    onCommit={(v) => set({ opacity: v }, "opacity")}
                  />
                  <label className="select-row">
                    <span>Blend</span>
                    <select
                      value={str(d.blendMode, "normal")}
                      data-testid="insp-blend"
                      onChange={(e) =>
                        set({ blendMode: e.target.value }, "blend mode")
                      }
                    >
                      {BLENDS.map((b) => (
                        <option key={b} value={b}>
                          {b}
                        </option>
                      ))}
                    </select>
                  </label>
                  <Slider
                    label="Corner radius"
                    value={num(d.borderRadius, 0)}
                    min={0}
                    max={200}
                    onCommit={(v) => set({ borderRadius: v }, "corner radius")}
                  />
                  <Slider
                    label="Blur"
                    value={num(d.gaussianBlur, 0)}
                    min={0}
                    max={50}
                    onCommit={(v) => set({ gaussianBlur: v }, "blur")}
                  />
                </Section>
                {(item.kind === "video" || item.kind === "image") && (
                  <Section
                    title="Crop"
                    aside={
                      <button
                        className="link"
                        onClick={() => unset(["crop"], "reset crop")}
                      >
                        Reset
                      </button>
                    }
                  >
                    <div className="grid2">
                      {(["top", "right", "bottom", "left"] as const).map(
                        (k) => (
                          <NumberField
                            key={k}
                            label={k[0].toUpperCase() + k.slice(1)}
                            unit="%"
                            value={num(crop[k], 0)}
                            onCommit={(v) =>
                              setIn(
                                "crop",
                                k,
                                Math.min(100, Math.max(0, v)),
                                "crop",
                              )
                            }
                          />
                        ),
                      )}
                    </div>
                  </Section>
                )}
              </>
            )}
            <Section title="Transitions">
              {catalog ? (
                <>
                  <TransitionEditor
                    catalog={catalog}
                    fields={d}
                    field="transitionIn"
                    title="In"
                    set={set}
                    unset={unset}
                  />
                  <TransitionEditor
                    catalog={catalog}
                    fields={d}
                    field="transitionOut"
                    title="Out"
                    set={set}
                    unset={unset}
                  />
                </>
              ) : (
                <>
                  {transitionRow("transitionIn", "In")}
                  {transitionRow("transitionOut", "Out")}
                </>
              )}
            </Section>
            {catalog &&
              (item.kind === "video" ||
                item.kind === "image" ||
                item.kind === "text") && (
                <Section
                  title="Layout"
                  aside={
                    d.layout !== undefined ? (
                      <button
                        className="link"
                        onClick={() => unset(["layout"], "leave layout")}
                      >
                        Remove
                      </button>
                    ) : undefined
                  }
                >
                  <label className="select-row">
                    <span>Preset</span>
                    <select
                      value={str(obj(d.layout).preset, "")}
                      data-testid="insp-layout"
                      onChange={(e) =>
                        e.target.value
                          ? set(
                              {
                                layout: {
                                  slot: 0,
                                  ...obj(d.layout),
                                  preset: e.target.value,
                                },
                              },
                              "layout",
                            )
                          : unset(["layout"], "leave layout")
                      }
                    >
                      <option value="">None (free placement)</option>
                      {catalog.layouts.map((l) => (
                        <option key={l.id} value={l.id}>
                          {l.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  {d.layout !== undefined && (
                    <div className="chips-row">
                      {(
                        catalog.layouts.find(
                          (l) => l.id === obj(d.layout).preset,
                        )?.slots ?? []
                      ).map((_, i) => (
                        <button
                          key={i}
                          className={`chip-sm${num(obj(d.layout).slot, 0) === i ? " on" : ""}`}
                          data-testid={`insp-layout-slot-${i}`}
                          onClick={() =>
                            setIn("layout", "slot", i, "layout slot")
                          }
                        >
                          Slot {i + 1}
                        </button>
                      ))}
                    </div>
                  )}
                </Section>
              )}
          </>
        )}
        {detail !== null && tab === "effects" && catalog && (
          <Section title="Effects">
            <EffectStackEditor catalog={catalog} fields={d} set={set} />
          </Section>
        )}
        {detail !== null && tab === "shader" && catalog && (
          <Section title="Shader">
            <ShaderEditor catalog={catalog} fields={d} set={set} />
          </Section>
        )}
        {detail !== null && tab === "color" && catalog && (
          <Section
            title="Grade"
            aside={
              <button
                className="link"
                onClick={() => unset(["colorGrade"], "reset grade")}
              >
                Reset
              </button>
            }
          >
            <ColorGradeEditor
              catalog={catalog}
              itemId={item.id}
              fields={d}
              playhead={playhead}
              set={set}
              onEdit={onEdit}
            />
          </Section>
        )}
        {detail !== null && tab === "color" && (
          <Section
            title="Adjust"
            aside={
              <button
                className="link"
                onClick={() => unset(["colorGrading"], "reset color")}
              >
                Reset
              </button>
            }
          >
            <Slider
              label="Exposure"
              value={num(grading.exposure, 0)}
              min={-3}
              max={3}
              step={0.05}
              testId="insp-exposure"
              onCommit={(v) => setIn("colorGrading", "exposure", v, "exposure")}
            />
            {(
              [
                "brightness",
                "contrast",
                "highlights",
                "shadows",
                "saturation",
                "vibrance",
                "temperature",
                "tint",
              ] as const
            ).map((k) => (
              <Slider
                key={k}
                label={k[0].toUpperCase() + k.slice(1)}
                value={num(grading[k], 0)}
                min={-100}
                max={100}
                testId={`insp-${k}`}
                onCommit={(v) => setIn("colorGrading", k, v, k)}
              />
            ))}
            {item.kind === "adjustment" && (
              <Slider
                label="Strength"
                value={num(d.lutIntensity, 1)}
                min={0}
                max={1}
                step={0.01}
                format={(v) => `${Math.round(v * 100)}%`}
                onCommit={(v) => set({ lutIntensity: v }, "strength")}
              />
            )}
          </Section>
        )}
        {detail !== null && tab === "audio" && (
          <>
            <Section title="Volume">
              <Slider
                label="Volume"
                value={num(d.volume, 100)}
                min={0}
                max={200}
                format={(v) => `${Math.round(v)}%`}
                testId="insp-volume"
                onCommit={(v) => set({ volume: v }, "volume")}
              />
              <Toggle
                label="Mute"
                on={d.muted === true}
                testId="insp-mute-toggle"
                onChange={(v) => set({ muted: v }, v ? "mute" : "unmute")}
              />
            </Section>
            <Section title="Fades">
              <Slider
                label="Fade in"
                value={num(obj(d.audioFadeIn).duration, 0)}
                min={0}
                max={5}
                step={0.1}
                format={(v) => `${v.toFixed(1)}s`}
                testId="insp-fade-in"
                onCommit={(v) =>
                  v > 0
                    ? set({ audioFadeIn: { duration: v } }, "fade in")
                    : unset(["audioFadeIn"], "no fade in")
                }
              />
              <Slider
                label="Fade out"
                value={num(obj(d.audioFadeOut).duration, 0)}
                min={0}
                max={5}
                step={0.1}
                format={(v) => `${v.toFixed(1)}s`}
                testId="insp-fade-out"
                onCommit={(v) =>
                  v > 0
                    ? set({ audioFadeOut: { duration: v } }, "fade out")
                    : unset(["audioFadeOut"], "no fade out")
                }
              />
            </Section>
            {item.kind === "audio" && (
              <Section title="Speed">
                <div className="chips-row">
                  {SPEEDS.map((s) => (
                    <button
                      key={s}
                      className={`chip-sm${num(d.playbackSpeed, 1) === s ? " on" : ""}`}
                      onClick={() => set({ playbackSpeed: s }, "speed")}
                    >
                      {s}×
                    </button>
                  ))}
                </div>
              </Section>
            )}
          </>
        )}
        {detail !== null && tab === "text" && (
          <>
            <Section title="Content">
              <TextArea
                value={str(d.textContent, "")}
                testId="insp-text"
                onCommit={(v) =>
                  set(
                    { textContent: v, name: v.split("\n")[0].slice(0, 40) },
                    "edit text",
                  )
                }
              />
            </Section>
            <Section title="Font">
              <FontPicker
                value={str(d.fontFamily, "Inter")}
                onChange={(f) => set({ fontFamily: f }, "font")}
              />
              <div className="grid2">
                <NumberField
                  label="Size"
                  unit="px"
                  value={num(d.fontSize, 64)}
                  testId="insp-font-size"
                  onCommit={(v) =>
                    set({ fontSize: Math.max(1, v) }, "font size")
                  }
                />
                <label className="select-row compact">
                  <span>Weight</span>
                  <select
                    value={String(d.fontWeight ?? 400)}
                    onChange={(e) =>
                      set({ fontWeight: Number(e.target.value) }, "font weight")
                    }
                  >
                    {[100, 200, 300, 400, 500, 600, 700, 800, 900].map((w) => (
                      <option key={w} value={w}>
                        {w}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <div className="chips-row">
                {(["left", "center", "right"] as const).map((a) => (
                  <button
                    key={a}
                    className={`chip-sm${str(d.textAlign, "center") === a ? " on" : ""}`}
                    data-testid={`insp-align-${a}`}
                    onClick={() => set({ textAlign: a }, "align")}
                  >
                    {a}
                  </button>
                ))}
                <button
                  className={`chip-sm${d.fontStyle === "italic" ? " on" : ""}`}
                  onClick={() =>
                    d.fontStyle === "italic"
                      ? unset(["fontStyle"], "upright")
                      : set({ fontStyle: "italic" }, "italic")
                  }
                >
                  <em>italic</em>
                </button>
              </div>
              <ColorField
                label="Color"
                value={str(d.color, "#ffffff")}
                onCommit={(v) => set({ color: v }, "text color")}
              />
              <Slider
                label="Line height"
                value={num(d.lineHeight, 1.2)}
                min={0.6}
                max={3}
                step={0.05}
                onCommit={(v) => set({ lineHeight: v }, "line height")}
              />
              <Slider
                label="Letter spacing"
                value={num(d.letterSpacing, 0)}
                min={-10}
                max={40}
                step={0.5}
                onCommit={(v) => set({ letterSpacing: v }, "letter spacing")}
              />
            </Section>
            <Section
              title="Background"
              aside={
                <button
                  className="link"
                  onClick={() =>
                    unset(
                      [
                        "backgroundColor",
                        "backgroundOpacity",
                        "backgroundPadding",
                      ],
                      "no background",
                    )
                  }
                >
                  None
                </button>
              }
            >
              <ColorField
                label="Color"
                value={str(d.backgroundColor, "#000000")}
                onCommit={(v) =>
                  set(
                    {
                      backgroundColor: v,
                      backgroundOpacity: num(d.backgroundOpacity, 0.6),
                    },
                    "background",
                  )
                }
              />
              <Slider
                label="Opacity"
                value={num(d.backgroundOpacity, 0)}
                min={0}
                max={1}
                step={0.01}
                format={(v) => `${Math.round(v * 100)}%`}
                onCommit={(v) =>
                  set(
                    {
                      backgroundOpacity: v,
                      backgroundColor: str(d.backgroundColor, "#000000"),
                    },
                    "background opacity",
                  )
                }
              />
              <Slider
                label="Padding"
                value={num(d.backgroundPadding, 0)}
                min={0}
                max={80}
                onCommit={(v) =>
                  set({ backgroundPadding: v }, "background padding")
                }
              />
            </Section>
            <Section title="Outline & shadow">
              <ColorField
                label="Outline"
                value={str(d.strokeColor, "#000000")}
                onCommit={(v) =>
                  set(
                    { strokeColor: v, strokeWidth: num(d.strokeWidth, 2) },
                    "outline",
                  )
                }
              />
              <Slider
                label="Outline width"
                value={num(d.strokeWidth, 0)}
                min={0}
                max={20}
                step={0.5}
                onCommit={(v) => set({ strokeWidth: v }, "outline width")}
              />
              <ColorField
                label="Shadow"
                value={str(d.shadowColor, "#000000")}
                onCommit={(v) =>
                  set(
                    { shadowColor: v, shadowBlur: num(d.shadowBlur, 8) },
                    "shadow",
                  )
                }
              />
              <Slider
                label="Shadow blur"
                value={num(d.shadowBlur, 0)}
                min={0}
                max={40}
                onCommit={(v) => set({ shadowBlur: v }, "shadow blur")}
              />
            </Section>
          </>
        )}
        {detail !== null && tab === "caption" && (
          <CaptionEditor
            detail={d}
            set={set}
            onStyle={(id) => onEdit(commands.applyCaptionStyle(id, [item.id]))}
          />
        )}
        {detail !== null && tab === "code" && (
          <CodeTab project={project} item={item} detail={d} set={set} />
        )}
      </div>
    </aside>
  );
}

type Segment = CaptionSegment;

function CaptionEditor({
  detail,
  set,
  onStyle,
}: {
  detail: Fields;
  set: (f: Fields, summary?: string) => void;
  onStyle: (presetId: string) => void;
}) {
  const segments: Segment[] = Array.isArray(detail.captionSegments)
    ? (detail.captionSegments as Segment[])
    : [];
  const save = (next: Segment[], summary: string) =>
    set(
      { captionSegments: next, captionText: next.map((s) => s.text).join(" ") },
      summary,
    );
  const presets = useCaptionPresets();
  const presetId = str(detail.captionPresetId, DEFAULT_CAPTION_PRESET);
  const preset =
    presets.find((p) => p.id.toLowerCase() === presetId.toLowerCase()) ??
    presets.find((p) => p.id === DEFAULT_CAPTION_PRESET);
  return (
    <>
      <Section title="Style">
        <label className="select-row">
          Style
          <select
            value={preset?.id ?? presetId}
            data-testid="insp-caption-style"
            onChange={(e) => onStyle(e.target.value)}
          >
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label} — {animationLabel(p.animation)}
              </option>
            ))}
          </select>
        </label>
        {preset && (
          <div className="preset-preview insp-caption-preview">
            <CaptionSample preset={preset} />
          </div>
        )}
        <ColorField
          label="Text"
          value={str(detail.captionTextColor, preset?.textColor ?? "#ffffff")}
          onCommit={(v) => set({ captionTextColor: v }, "caption color")}
        />
        <ColorField
          label="Highlight"
          value={str(
            detail.captionHighlightColor,
            preset?.highlightColor ?? "#ffd84d",
          )}
          onCommit={(v) =>
            set({ captionHighlightColor: v }, "caption highlight")
          }
        />
        <NumberField
          label="Size"
          unit="px"
          value={num(detail.captionFontSize, preset?.fontSize ?? 56)}
          onCommit={(v) =>
            set({ captionFontSize: Math.max(1, v) }, "caption size")
          }
        />
        <label className="select-row">
          Position
          <select
            value={str(detail.captionPosition, preset?.position ?? "bottom")}
            data-testid="insp-caption-position"
            onChange={(e) =>
              set({ captionPosition: e.target.value }, "caption position")
            }
          >
            <option value="top">Top</option>
            <option value="center">Center</option>
            <option value="bottom">Bottom</option>
          </select>
        </label>
      </Section>
      <Section title="Transcript">
        <p className="muted small">
          Generate captions from the sound in the Captions panel, or type the
          words here.
        </p>
        {segments.map((s, i) => (
          <div
            className="segment"
            key={s.id ?? i}
            data-testid="caption-segment"
          >
            <div className="segment-times">
              <NumberField
                label=""
                unit="s"
                value={num(s.startTime, 0)}
                step={0.1}
                onCommit={(v) =>
                  save(
                    segments.map((x, j) =>
                      j === i ? { ...x, startTime: Math.max(0, v) } : x,
                    ),
                    "caption timing",
                  )
                }
              />
              <NumberField
                label=""
                unit="s"
                value={num(s.endTime, 0)}
                step={0.1}
                onCommit={(v) =>
                  save(
                    segments.map((x, j) =>
                      j === i
                        ? { ...x, endTime: Math.max(x.startTime + 0.1, v) }
                        : x,
                    ),
                    "caption timing",
                  )
                }
              />
              <button
                className="icon-btn"
                title="Remove segment"
                onClick={() =>
                  save(
                    segments.filter((_, j) => j !== i),
                    "remove caption segment",
                  )
                }
              >
                <Icon name="close" size={14} />
              </button>
            </div>
            <TextArea
              rows={2}
              value={s.text}
              onCommit={(v) =>
                save(
                  segments.map((x, j) => (j === i ? retimeSegment(x, v) : x)),
                  "edit caption",
                )
              }
            />
          </div>
        ))}
        <button
          className="btn"
          data-testid="caption-add-segment"
          onClick={() => {
            const last = segments[segments.length - 1];
            const start = last ? last.endTime : 0;
            save(
              [
                ...segments,
                {
                  id: `s${Date.now().toString(36)}`,
                  text: "New line",
                  startTime: start,
                  endTime: start + 2,
                },
              ],
              "add caption segment",
            );
          }}
        >
          <Icon name="plus" size={14} /> Add line
        </button>
      </Section>
    </>
  );
}
