// Guides drawn over the viewer (add-viewer-interaction-and-text-fills task 3.1) and the menu that turns them on. The
// layer ignores the pointer; it sits under the handles. Nothing here reaches the project or an export.

import { useEffect, useRef, useState } from "react";
import { Icon } from "../icons";
import {
  ACTION_SAFE,
  type Guides,
  MASKS,
  type Mask,
  NO_GUIDES,
  type Preset,
  TITLE_SAFE,
  anyGuide,
  cleanPreset,
  gridLines,
  maskRect,
} from "./guides";

export function GuideLayer({
  guides,
  presets,
  width,
  height,
}: {
  guides: Guides;
  presets: Preset[];
  width: number;
  height: number;
}) {
  if (!anyGuide(guides) || width <= 0 || height <= 0) return null;
  const inset = (f: number) => ({ x: width * f, y: height * f, w: width * (1 - 2 * f), h: height * (1 - 2 * f) });
  const lines: { x1: number; y1: number; x2: number; y2: number; key: string; cls: string }[] = [];
  const grid = (n: number, cls: string) => {
    for (const f of gridLines(n)) {
      lines.push({ key: `${cls}v${f}`, cls, x1: f * width, y1: 0, x2: f * width, y2: height });
      lines.push({ key: `${cls}h${f}`, cls, x1: 0, y1: f * height, x2: width, y2: f * height });
    }
  };
  if (guides.thirds) grid(3, "guide-thirds");
  if (guides.grid >= 2) grid(Math.min(5, guides.grid), "guide-grid");
  const platform = presets.find((p) => p.id === guides.platform);
  const m = guides.mask ? maskRect(guides.mask, width, height) : null;
  const a = inset(ACTION_SAFE);
  const t = inset(TITLE_SAFE);
  return (
    <svg
      className="guide-layer"
      data-testid="guide-layer"
      width={width}
      height={height}
      style={{ position: "absolute", inset: 0, pointerEvents: "none", zIndex: 1 }}
    >
      {m && (
        <g data-testid="guide-mask" data-mask={guides.mask ?? ""}>
          <path
            className="guide-mask"
            fillRule="evenodd"
            d={`M0 0H${width}V${height}H0Z M${m.x} ${m.y}H${m.x + m.w}V${m.y + m.h}H${m.x}Z`}
          />
          <rect className="guide-mask-edge" x={m.x} y={m.y} width={m.w} height={m.h} />
        </g>
      )}
      {lines.map((l) => (
        <line key={l.key} className={l.cls} x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} />
      ))}
      {guides.actionSafe && <rect className="guide-action" data-testid="guide-action" x={a.x} y={a.y} width={a.w} height={a.h} />}
      {guides.titleSafe && <rect className="guide-title" data-testid="guide-title" x={t.x} y={t.y} width={t.w} height={t.h} />}
      {guides.centre && (
        <g className="guide-centre" data-testid="guide-centre">
          <line x1={width / 2 - 12} y1={height / 2} x2={width / 2 + 12} y2={height / 2} />
          <line x1={width / 2} y1={height / 2 - 12} x2={width / 2} y2={height / 2 + 12} />
        </g>
      )}
      {platform && (
        <rect
          className="guide-platform"
          data-testid="guide-platform"
          x={(width * platform.left) / 100}
          y={(height * platform.top) / 100}
          width={(width * (100 - platform.left - platform.right)) / 100}
          height={(height * (100 - platform.top - platform.bottom)) / 100}
        />
      )}
    </svg>
  );
}

/** The guides menu of the viewer toolbar, with the platform preset editor. */
export function GuidesMenu({
  guides,
  presets,
  onChange,
  onPresets,
  testId = "guides",
}: {
  guides: Guides;
  presets: Preset[];
  onChange: (g: Guides) => void;
  onPresets: (p: Preset[]) => void;
  testId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  const set = (patch: Partial<Guides>) => onChange({ ...guides, ...patch });
  const check = (id: string, label: string, on: boolean, flip: () => void) => (
    <label className="guide-option" key={id}>
      <input type="checkbox" data-testid={`${testId}-${id}`} checked={on} onChange={flip} />
      {label}
    </label>
  );
  const editPreset = (i: number, patch: Partial<Preset>) =>
    onPresets(presets.map((p, j) => (j === i ? cleanPreset({ ...p, ...patch }) : p)));
  return (
    <div className="dropdown guides-menu" ref={root}>
      <button
        className={`pill${anyGuide(guides) ? " on" : ""}`}
        data-testid={`${testId}-button`}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Guides, safe areas and format masks (never exported)"
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="layout" size={14} />
        Guides
      </button>
      {open && (
        <div className="menu menu-right menu-down guides-panel" data-testid={`${testId}-menu`}>
          {check("thirds", "Rule of thirds", guides.thirds, () => set({ thirds: !guides.thirds }))}
          <label className="guide-option">
            Grid
            <select
              data-testid={`${testId}-grid`}
              value={guides.grid}
              onChange={(e) => set({ grid: Number(e.target.value) })}
            >
              <option value={0}>Off</option>
              {[2, 3, 4, 5].map((n) => (
                <option key={n} value={n}>
                  {n} × {n}
                </option>
              ))}
            </select>
          </label>
          {check("action", "Action safe (93 %)", guides.actionSafe, () => set({ actionSafe: !guides.actionSafe }))}
          {check("title", "Title safe (90 %)", guides.titleSafe, () => set({ titleSafe: !guides.titleSafe }))}
          {check("centre", "Centre cross", guides.centre, () => set({ centre: !guides.centre }))}
          <label className="guide-option">
            Format mask
            <select
              data-testid={`${testId}-mask`}
              value={guides.mask ?? ""}
              onChange={(e) => set({ mask: (e.target.value || null) as Mask | null })}
            >
              <option value="">Off</option>
              {MASKS.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </label>
          <label className="guide-option">
            Platform zone
            <select
              data-testid={`${testId}-platform`}
              value={guides.platform ?? ""}
              onChange={(e) => set({ platform: e.target.value || null })}
            >
              <option value="">Off</option>
              {presets.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          <div className="guide-actions">
            <button className="chip-sm" data-testid={`${testId}-edit-presets`} onClick={() => setEditing((e) => !e)}>
              {editing ? "Done" : "Edit zones…"}
            </button>
            <button className="chip-sm" data-testid={`${testId}-off`} onClick={() => onChange(NO_GUIDES)}>
              All off
            </button>
          </div>
          {editing && (
            <div className="guide-presets" data-testid={`${testId}-presets`}>
              <div className="muted">Percent of the frame the platform's interface covers on each side.</div>
              {presets.map((p, i) => (
                <div className="guide-preset" key={p.id}>
                  <input
                    aria-label="Zone name"
                    data-testid={`${testId}-preset-name-${i}`}
                    value={p.name}
                    onChange={(e) => editPreset(i, { name: e.target.value })}
                  />
                  {(["top", "right", "bottom", "left"] as const).map((side) => (
                    <label key={side}>
                      {side}
                      <input
                        type="number"
                        min={0}
                        max={90}
                        step={0.5}
                        data-testid={`${testId}-preset-${side}-${i}`}
                        value={p[side]}
                        onChange={(e) => editPreset(i, { [side]: Number(e.target.value) })}
                      />
                    </label>
                  ))}
                  <button
                    className="icon-btn"
                    title="Remove this zone"
                    aria-label="Remove this zone"
                    onClick={() => onPresets(presets.filter((_, j) => j !== i))}
                  >
                    <Icon name="close" size={12} />
                  </button>
                </div>
              ))}
              <button
                className="chip-sm"
                data-testid={`${testId}-preset-add`}
                onClick={() =>
                  onPresets([
                    ...presets,
                    { id: `zone-${Date.now().toString(36)}`, name: "New zone", top: 10, right: 10, bottom: 10, left: 10 },
                  ])
                }
              >
                Add zone
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
