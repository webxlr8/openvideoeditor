// Inspector editors for change 12: effect stacks with parameter editors generated from the core's contracts,
// shader items, transitions (type, duration, easing, direction), and the colour grade (wheels, curves,
// saturation, LUT, match, scopes). Each gesture commits one revision.

import { useEffect, useRef, useState } from "react";
import { commands } from "./bridge";
import type { EditResult, Fields } from "./edit";
import {
  type EffectEntry,
  type EffectSpec,
  type FxCatalog,
  type ParamSpec,
  GROUP_LABEL,
  effectStack,
  hexToRgb,
  newEffect,
  paramValue,
  rgbToHex,
  splitTransition,
  usePreview,
} from "./fx";
import { Icon } from "./icons";
import { Slider } from "./Inspector";
import { fire, runCommand } from "./notify";

const obj = (v: unknown): Fields => (v && typeof v === "object" && !Array.isArray(v) ? (v as Fields) : {});
const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);
const str = (v: unknown, d: string) => (typeof v === "string" ? v : d);
const pct = (v: number) => `${Math.round(v * 100)}%`;
const label = (name: string) => name.replace(/([A-Z])/g, " $1").replace(/^./, (c) => c.toUpperCase());

// ---------------------------------------------------------------------------------------------------------------
// Parameters

function PointPad({ value, onCommit, testId }: { value: number[]; onCommit: (v: number[]) => void; testId?: string }) {
  const [x, y] = [num(value[0], 0.5), num(value[1], 0.5)];
  const pick = (e: React.PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const nx = Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width)));
    const ny = Math.min(1, Math.max(0, (e.clientY - r.top) / Math.max(1, r.height)));
    onCommit([Math.round(nx * 1000) / 1000, Math.round(ny * 1000) / 1000]);
  };
  return (
    <div className="point-pad" data-testid={testId} onPointerDown={pick} title={`${x.toFixed(2)}, ${y.toFixed(2)}`}>
      <span className="point-dot" style={{ left: `${x * 100}%`, top: `${y * 100}%` }} />
    </div>
  );
}

export function ParamEditor({
  spec,
  values,
  onCommit,
  testPrefix,
}: {
  spec: ParamSpec;
  values: Fields;
  onCommit: (name: string, v: unknown) => void;
  testPrefix: string;
}) {
  const v = paramValue(values, spec);
  const k = spec.kind;
  const id = `${testPrefix}-${spec.name}`;
  switch (k.kind) {
    case "number":
    case "percent": {
      const span = k.max - k.min;
      const step = span <= 0.05 ? 0.0005 : span <= 2 ? 0.01 : span <= 20 ? 0.1 : 1;
      return (
        <Slider
          label={label(spec.name)}
          value={num(v, k.min)}
          min={k.min}
          max={k.max}
          step={step}
          format={k.kind === "percent" && k.max <= 1 ? pct : undefined}
          testId={id}
          onCommit={(x) => onCommit(spec.name, x)}
        />
      );
    }
    case "color":
      return (
        <label className="color-field">
          <span>{label(spec.name)}</span>
          <input type="color" value={rgbToHex(v)} data-testid={id} onChange={(e) => onCommit(spec.name, hexToRgb(e.target.value))} />
          <code>{rgbToHex(v)}</code>
        </label>
      );
    case "point":
      return (
        <div className="param-point">
          <span>{label(spec.name)}</span>
          <PointPad value={Array.isArray(v) ? (v as number[]) : [0.5, 0.5]} testId={id} onCommit={(p) => onCommit(spec.name, p)} />
        </div>
      );
    case "toggle":
      return (
        <label className="check-row">
          <input type="checkbox" checked={v === true || v === 1} data-testid={id} onChange={(e) => onCommit(spec.name, e.target.checked)} />
          <span>{label(spec.name)}</span>
        </label>
      );
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Effect stack

function EffectThumb({ id }: { id: string }) {
  const url = usePreview("effect", id);
  return url ? <img className="fx-thumb" src={url} alt="" /> : <span className="fx-thumb placeholder" />;
}

export function EffectStackEditor({ catalog, fields, set }: { catalog: FxCatalog; fields: Fields; set: (f: Fields, summary?: string) => void }) {
  const stack = effectStack(fields);
  const [adding, setAdding] = useState("");
  const save = (next: EffectEntry[], summary: string) => set({ effects: next }, summary);
  const move = (i: number, d: number) => {
    const next = [...stack];
    const [x] = next.splice(i, 1);
    next.splice(Math.min(next.length, Math.max(0, i + d)), 0, x);
    save(next, "reorder effects");
  };
  return (
    <div className="fx-stack" data-testid="fx-stack">
      {stack.length === 0 && <p className="muted small">No effects. Add one below or from the Effects panel.</p>}
      {stack.map((e, i) => {
        const spec = catalog.effects.find((s) => s.id === e.type);
        const values = obj(e.params ?? e);
        const enabled = e.enabled !== false;
        const update = (name: string, v: unknown) =>
          save(
            stack.map((x, j) => (j === i ? (x.params ? { ...x, params: { ...obj(x.params), [name]: v } } : { ...x, [name]: v }) : x)),
            `${spec?.label.toLowerCase() ?? "effect"} ${label(name).toLowerCase()}`,
          );
        return (
          <div key={e.id ?? i} className={`fx-entry${enabled ? "" : " off"}`} data-testid={`fx-entry-${e.type}`}>
            <header>
              <EffectThumb id={e.type} />
              <strong>{spec?.label ?? e.type}</strong>
              <span className="fx-entry-tools">
                <button className="icon-btn" title="Move up" disabled={i === 0} onClick={() => move(i, -1)}>
                  ↑
                </button>
                <button className="icon-btn" title="Move down" disabled={i === stack.length - 1} onClick={() => move(i, 1)}>
                  ↓
                </button>
                <button
                  className={`chip-sm${enabled ? " on" : ""}`}
                  title={enabled ? "Turn off" : "Turn on"}
                  onClick={() => save(stack.map((x, j) => (j === i ? { ...x, enabled: !enabled } : x)), enabled ? "turn effect off" : "turn effect on")}
                >
                  {enabled ? "On" : "Off"}
                </button>
                <button className="icon-btn" title="Remove" data-testid={`fx-remove-${e.type}`} onClick={() => save(stack.filter((_, j) => j !== i), "remove effect")}>
                  <Icon name="close" size={14} />
                </button>
              </span>
            </header>
            <Slider label="Amount" value={num(e.amount, 1)} min={0} max={1} step={0.01} format={pct} onCommit={(v) => save(stack.map((x, j) => (j === i ? { ...x, amount: v } : x)), "effect amount")} />
            {spec?.params.map((p) => <ParamEditor key={p.name} spec={p} values={values} testPrefix={`fx-${e.type}`} onCommit={update} />)}
          </div>
        );
      })}
      <label className="select-row">
        <span>Add</span>
        <select
          value={adding}
          data-testid="fx-add"
          onChange={(ev) => {
            const spec = catalog.effects.find((s) => s.id === ev.target.value);
            setAdding("");
            if (spec) save([...stack, newEffect(spec)], `add ${spec.label.toLowerCase()}`);
          }}
        >
          <option value="">Effect…</option>
          {(["stylize", "distort", "background"] as const).map((g) => (
            <optgroup key={g} label={GROUP_LABEL[g]}>
              {catalog.effects
                .filter((s) => s.group === g)
                .map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label}
                  </option>
                ))}
            </optgroup>
          ))}
        </select>
      </label>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Shader items

/** Subject masks don't exist yet: the subject and background regions fall back to the whole frame. */
const REGIONS = [
  { id: "all", label: "Whole frame" },
  { id: "subject", label: "Subject (whole frame for now)" },
  { id: "background", label: "Background (whole frame for now)" },
];

export function ShaderEditor({ catalog, fields, set }: { catalog: FxCatalog; fields: Fields; set: (f: Fields, summary?: string) => void }) {
  const comp = obj(fields.shaderComposition);
  const spec: EffectSpec | undefined = catalog.effects.find((s) => s.id === comp.template);
  const params = obj(comp.params);
  const put = (patch: Fields, summary: string) => set({ shaderComposition: { ...comp, ...patch } }, summary);
  return (
    <div data-testid="shader-editor">
      <label className="select-row">
        <span>Effect</span>
        <select value={str(comp.template, "")} data-testid="shader-template" onChange={(e) => put({ template: e.target.value, params: {} }, "shader effect")}>
          {catalog.effects.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label}
            </option>
          ))}
        </select>
      </label>
      <Slider label="Amount" value={num(comp.amount, 100) > 1 ? num(comp.amount, 100) / 100 : num(comp.amount, 1)} min={0} max={1} step={0.01} format={pct} testId="shader-amount" onCommit={(v) => put({ amount: Math.round(v * 100) }, "shader amount")} />
      <Slider label="Fade in" value={num(comp.fadeIn, 0)} min={0} max={5} step={0.1} format={(v) => `${v.toFixed(1)}s`} onCommit={(v) => put({ fadeIn: v }, "shader fade in")} />
      <Slider label="Fade out" value={num(comp.fadeOut, 0)} min={0} max={5} step={0.1} format={(v) => `${v.toFixed(1)}s`} onCommit={(v) => put({ fadeOut: v }, "shader fade out")} />
      <label className="select-row">
        <span>Region</span>
        <select
          value={str(comp.region, "all")}
          data-testid="shader-region"
          title="Subject and background need subject detection, which isn't available yet; until then they apply to the whole frame."
          onChange={(e) => put({ region: e.target.value }, "shader region")}
        >
          {REGIONS.map((r) => (
            <option key={r.id} value={r.id}>
              {r.label}
            </option>
          ))}
        </select>
      </label>
      {typeof comp.region === "string" && comp.region !== "all" && (
        <p className="muted small" data-testid="shader-region-note">
          Subject and background regions apply to the whole frame until subject detection is available.
        </p>
      )}
      {spec?.params.map((p) => (
        <ParamEditor key={p.name} spec={p} values={params} testPrefix="shader" onCommit={(name, v) => put({ params: { ...params, [name]: v } }, `shader ${label(name).toLowerCase()}`)} />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Transitions

export function TransitionEditor({
  catalog,
  fields,
  field,
  title,
  set,
  unset,
}: {
  catalog: FxCatalog;
  fields: Fields;
  field: "transitionIn" | "transitionOut";
  title: string;
  set: (f: Fields, summary?: string) => void;
  unset: (keys: string[], summary?: string) => void;
}) {
  const t = obj(fields[field]);
  const type = typeof t.type === "string" ? t.type : "";
  const known = catalog.transitions.some((x) => x.id === type);
  const spec = catalog.transitions.find((x) => x.id === type);
  const put = (patch: Fields, summary: string) => set({ [field]: { duration: 0.5, ...t, ...patch } }, summary);
  return (
    <div className="transition-edit" data-testid={`insp-${field}-editor`}>
      <div className="transition-row">
        <span>{title}</span>
        <select
          value={type || "none"}
          data-testid={`insp-${field}`}
          onChange={(e) =>
            e.target.value === "none" ? unset([field], `remove ${title.toLowerCase()} transition`) : put({ type: e.target.value }, `${title.toLowerCase()} transition`)
          }
        >
          <option value="none">None</option>
          {!known && type && <option value={type}>{type}</option>}
          {catalog.transitions.map((x) => (
            <option key={x.id} value={x.id}>
              {x.label}
            </option>
          ))}
        </select>
      </div>
      {type && (
        <>
          <Slider label="Duration" value={num(t.duration, 0.5)} min={0.1} max={3} step={0.05} format={(v) => `${v.toFixed(2)}s`} testId={`insp-${field}-duration`} onCommit={(v) => put({ duration: v }, "transition duration")} />
          <label className="select-row">
            <span>Easing</span>
            <select value={str(t.easing, "linear")} data-testid={`insp-${field}-easing`} onChange={(e) => put({ easing: e.target.value }, "transition easing")}>
              {catalog.easings.map((x) => (
                <option key={x}>{x}</option>
              ))}
            </select>
          </label>
          {spec?.directional && (
            <div className="chips-row" role="group" aria-label="Direction">
              {catalog.directions.map((d) => (
                <button key={d} className={`chip-sm${splitTransition(type, t.direction).direction === d ? " on" : ""}`} data-testid={`insp-${field}-dir-${d}`} onClick={() => put({ direction: d }, "transition direction")}>
                  {d}
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Colour: wheels, curves, LUT, match, scopes

interface Wheel {
  r: number;
  g: number;
  b: number;
  y: number;
}

const wheelOf = (v: unknown): Wheel => {
  const o = obj(v);
  return { r: num(o.r, 0), g: num(o.g, 0), b: num(o.b, 0), y: num(o.y, 0) };
};

// A balanced colour offset lives in the plane r+g+b = 0: map it to the wheel's x/y and back.
const SQ3 = Math.sqrt(3);
const toXY = (w: Wheel) => ({ x: w.r - (w.g + w.b) / 2, y: (SQ3 / 2) * (w.g - w.b) });
const fromXY = (x: number, y: number) => {
  const r = (2 / 3) * x;
  return { r, g: -x / 3 + y / SQ3, b: -x / 3 - y / SQ3 };
};

/** A colour wheel (drag the puck to tint) with a master slider. */
export function ColorWheel({ name, value, range, onCommit }: { name: string; value: Wheel; range: number; onCommit: (w: Wheel) => void }) {
  const [live, setLive] = useState<Wheel | null>(null);
  const w = live ?? value;
  const xy = toXY(w);
  // Values past the rim (typed or fitted) show on the rim.
  const over = Math.max(1, Math.hypot(xy.x, xy.y) / range);
  const [x, y] = [xy.x / over, xy.y / over];
  const at = (e: React.PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    let nx = ((e.clientX - r.left) / Math.max(1, r.width)) * 2 - 1;
    let ny = -(((e.clientY - r.top) / Math.max(1, r.height)) * 2 - 1);
    const len = Math.hypot(nx, ny);
    if (len > 1) [nx, ny] = [nx / len, ny / len];
    const c = fromXY(nx * range, ny * range);
    const round = (v: number) => Math.round(v * 1000) / 1000;
    return { ...w, r: round(c.r), g: round(c.g), b: round(c.b) };
  };
  return (
    <div className="wheel" data-testid={`wheel-${name}`}>
      <div
        className="wheel-disc"
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture?.(e.pointerId);
          setLive(at(e));
        }}
        onPointerMove={(e) => live && setLive(at(e))}
        onPointerUp={() => {
          if (live) onCommit(live);
          setLive(null);
        }}
        onDoubleClick={() => onCommit({ ...w, r: 0, g: 0, b: 0 })}
        title="Drag to tint; double-click to reset"
      >
        <span className="wheel-puck" style={{ left: `${50 + (x / range) * 50}%`, top: `${50 - (y / range) * 50}%` }} />
      </div>
      <strong>{label(name)}</strong>
      <Slider label="" value={w.y} min={-1} max={1} step={0.01} testId={`wheel-${name}-y`} onCommit={(v) => onCommit({ ...w, y: v })} />
    </div>
  );
}

type Pt = [number, number];
const ptsOf = (v: unknown): Pt[] =>
  Array.isArray(v)
    ? v
        .map((p) => (Array.isArray(p) ? [num(p[0], 0), num(p[1], 0)] : [num(obj(p).x, 0), num(obj(p).y, 0)]))
        .filter((p): p is Pt => p.length === 2)
        .sort((a, b) => a[0] - b[0])
    : [];

export const CURVES: { key: string; label: string; neutral: "diagonal" | "flat"; color: string }[] = [
  { key: "master", label: "Master", neutral: "diagonal", color: "#e8e8e8" },
  { key: "red", label: "Red", neutral: "diagonal", color: "#ff5a5a" },
  { key: "green", label: "Green", neutral: "diagonal", color: "#5ad16a" },
  { key: "blue", label: "Blue", neutral: "diagonal", color: "#5a8cff" },
  { key: "hueVsSat", label: "Hue vs sat", neutral: "flat", color: "#f0b400" },
  { key: "hueVsHue", label: "Hue vs hue", neutral: "flat", color: "#c070ff" },
  { key: "hueVsLum", label: "Hue vs lum", neutral: "flat", color: "#60d0d0" },
  { key: "lumVsSat", label: "Lum vs sat", neutral: "flat", color: "#ff9050" },
];

/** Monotone-ish preview path (straight segments are enough to read the shape). */
function curvePath(pts: Pt[], w: number, h: number): string {
  if (pts.length === 0) return "";
  const all: Pt[] = [[0, pts[0][1]], ...pts, [1, pts[pts.length - 1][1]]];
  return all.map((p, i) => `${i ? "L" : "M"}${(p[0] * w).toFixed(1)},${((1 - p[1]) * h).toFixed(1)}`).join(" ");
}

/** Click to add a point, drag to move, double-click a point to remove it. */
export function CurveEditor({ curve, value, onCommit }: { curve: (typeof CURVES)[number]; value: unknown; onCommit: (pts: Pt[] | null) => void }) {
  const base: Pt[] = curve.neutral === "diagonal" ? [[0, 0], [1, 1]] : [[0, 0.5], [1, 0.5]];
  const stored = ptsOf(value);
  const [live, setLive] = useState<Pt[] | null>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const pts = live ?? (stored.length ? stored : base);
  const ref = useRef<SVGSVGElement>(null);
  const W = 200;
  const H = 140;
  const pos = (e: React.PointerEvent): Pt => {
    const r = ref.current!.getBoundingClientRect();
    const x = Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width)));
    const y = Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / Math.max(1, r.height)));
    return [Math.round(x * 1000) / 1000, Math.round(y * 1000) / 1000];
  };
  return (
    <svg
      ref={ref}
      className="curve-editor"
      viewBox={`0 0 ${W} ${H}`}
      data-testid={`curve-${curve.key}`}
      onPointerDown={(e) => {
        if (drag !== null) return;
        const p = pos(e);
        const next = [...pts, p].sort((a, b) => a[0] - b[0]);
        setLive(next);
        setDrag(next.indexOf(p));
      }}
      onPointerMove={(e) => {
        if (drag === null || !live) return;
        const p = pos(e);
        setLive(live.map((q, i) => (i === drag ? p : q)));
      }}
      onPointerUp={() => {
        if (live) onCommit([...live].sort((a, b) => a[0] - b[0]));
        setLive(null);
        setDrag(null);
      }}
    >
      {[0.25, 0.5, 0.75].map((g) => (
        <g key={g} className="curve-grid">
          <line x1={g * W} y1={0} x2={g * W} y2={H} />
          <line x1={0} y1={g * H} x2={W} y2={g * H} />
        </g>
      ))}
      <path d={curvePath(pts, W, H)} stroke={curve.color} fill="none" strokeWidth={2} />
      {pts.map((p, i) => (
        <circle
          key={i}
          cx={p[0] * W}
          cy={(1 - p[1]) * H}
          r={5}
          fill={curve.color}
          onPointerDown={(e) => {
            e.stopPropagation();
            e.currentTarget.ownerSVGElement?.setPointerCapture?.(e.pointerId);
            setLive(pts);
            setDrag(i);
          }}
          onDoubleClick={(e) => {
            e.stopPropagation();
            const next = pts.filter((_, j) => j !== i);
            onCommit(next.length ? next : null);
          }}
        />
      ))}
    </svg>
  );
}

export function Histogram({ time }: { time: number }) {
  const [data, setData] = useState<number[][] | null>(null);
  const refresh = () => runCommand(commands.colorScopes(time), (d) => setData(d.map((c) => c.map((v) => v ?? 0))));
  useEffect(refresh, [time]);
  const W = 256;
  const H = 80;
  const colors = ["#ff5a5a", "#5ad16a", "#5a8cff", "#dddddd"];
  return (
    <div className="scopes" data-testid="scopes">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        {data?.map((ch, k) => (
          <path
            key={k}
            d={`M0,${H} ${ch.map((v, i) => `L${((i + 0.5) / ch.length) * W},${H - Math.sqrt(v) * H}`).join(" ")} L${W},${H} Z`}
            fill={colors[k]}
            fillOpacity={k === 3 ? 0.15 : 0.3}
            stroke={colors[k]}
            strokeOpacity={0.8}
          />
        ))}
      </svg>
      <button className="link" onClick={refresh}>
        Refresh
      </button>
    </div>
  );
}

function LutThumb({ id }: { id: string }) {
  const url = usePreview("lut", id);
  return url ? <img className="fx-thumb" src={url} alt="" /> : <span className="fx-thumb placeholder" />;
}

export function ColorGradeEditor({
  catalog,
  itemId,
  fields,
  playhead,
  set,
  onEdit,
}: {
  catalog: FxCatalog;
  itemId: string;
  fields: Fields;
  playhead: number;
  set: (f: Fields, summary?: string) => void;
  onEdit: (p: EditResult) => void;
}) {
  const g = obj(fields.colorGrade);
  const wheels = obj(g.wheels);
  const curves = obj(g.curves);
  const lut = obj(g.lut);
  const [curveKey, setCurveKey] = useState("master");
  const [matching, setMatching] = useState(false);
  const put = (patch: Fields, summary: string) => {
    const next: Fields = { ...g, ...patch };
    for (const [k, v] of Object.entries(next)) if (v === undefined) delete next[k];
    set({ colorGrade: next }, summary);
  };
  const curve = CURVES.find((c) => c.key === curveKey) ?? CURVES[0];
  return (
    <div className="grade" data-testid="grade-editor">
      <div className="wheels">
        {(["lift", "gamma", "gain", "offset"] as const).map((k) => (
          <ColorWheel key={k} name={k} value={wheelOf(wheels[k])} range={k === "offset" ? 0.5 : 1} onCommit={(w) => put({ wheels: { ...wheels, [k]: w } }, `${k} wheel`)} />
        ))}
      </div>
      <Slider label="Saturation" value={num(g.saturation, 1)} min={0} max={2} step={0.01} format={pct} testId="grade-saturation" onCommit={(v) => put({ saturation: v }, "saturation")} />
      <div className="curve-tabs" role="tablist">
        {CURVES.map((c) => (
          <button key={c.key} role="tab" className={`chip-sm${c.key === curveKey ? " on" : ""}${curves[c.key] ? " set" : ""}`} data-testid={`curve-tab-${c.key}`} onClick={() => setCurveKey(c.key)}>
            {c.label}
          </button>
        ))}
      </div>
      <CurveEditor
        key={curve.key}
        curve={curve}
        value={curves[curve.key]}
        onCommit={(pts) => {
          const next = { ...curves };
          if (pts) next[curve.key] = pts;
          else delete next[curve.key];
          put({ curves: Object.keys(next).length ? next : undefined }, `${curve.label.toLowerCase()} curve`);
        }}
      />
      <h5 className="panel-sub">LUT</h5>
      <div className="lut-grid">
        <button className={`lut-card${!lut.preset && !lut.file ? " on" : ""}`} onClick={() => put({ lut: undefined }, "no LUT")}>
          <span className="fx-thumb placeholder" />
          None
        </button>
        {catalog.luts.map((l) => (
          <button key={l.id} className={`lut-card${lut.preset === l.id && !lut.file ? " on" : ""}`} data-testid={`lut-${l.id}`} onClick={() => put({ lut: { intensity: 1, stage: "look", ...lut, file: undefined, preset: l.id } }, `${l.label.toLowerCase()} look`)}>
            <LutThumb id={l.id} />
            {l.label}
          </button>
        ))}
      </div>
      {typeof lut.file === "string" && <p className="muted small">Imported: {lut.file}</p>}
      <div className="chips-row">
        <button
          className="chip-sm"
          data-testid="lut-import"
          onClick={() =>
            runCommand(commands.importLut(null), (file) => {
              if (file) put({ lut: { intensity: 1, stage: "look", ...lut, preset: undefined, file } }, "import LUT");
            })
          }
        >
          Import .cube…
        </button>
      </div>
      {Boolean(lut.preset || lut.file) && (
        <>
          <Slider label="Intensity" value={num(lut.intensity, 1)} min={0} max={1} step={0.01} format={pct} testId="lut-intensity" onCommit={(v) => put({ lut: { ...lut, intensity: v } }, "LUT intensity")} />
          <label className="select-row">
            <span>Stage</span>
            <select value={str(lut.stage, "look")} data-testid="lut-stage" onChange={(e) => put({ lut: { ...lut, stage: e.target.value } }, "LUT stage")}>
              <option value="input">Input (before the grade)</option>
              <option value="look">Look (after the grade)</option>
            </select>
          </label>
        </>
      )}
      <>
          <h5 className="panel-sub">Match</h5>
          <p className="muted small">Fit this clip's colour to the frame under the playhead ({playhead.toFixed(2)}s).</p>
          <div className="chips-row">
            <button
              className="chip-sm"
              data-testid="grade-match"
              disabled={matching}
              onClick={() => {
                setMatching(true);
                const r = commands.matchColor(itemId, playhead, 1);
                onEdit(r);
                fire(r.finally(() => setMatching(false)));
              }}
            >
              {matching ? "Matching…" : "Match to frame"}
            </button>
            <button className="chip-sm" onClick={() => set({ colorGrade: {} }, "reset grade")}>
              Reset grade
            </button>
          </div>
          <h5 className="panel-sub">Scopes</h5>
          <Histogram time={playhead} />
      </>
    </div>
  );
}
