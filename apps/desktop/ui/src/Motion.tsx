// Motion graphics UI: the text-animation picker, the font picker with rendered previews, the Motion templates
// gallery and the Code inspector tab (template props form, source editor, diagnostics).

import { useEffect, useMemo, useRef, useState } from "react";
import type { FontFamily, ItemView_Serialize, ProjectView_Serialize } from "./bindings";
import { type EditResult, type Fields, addItems, firstTrack, snapToFrame, updateItems } from "./edit";
import { Icon } from "./icons";
import { fire } from "./notify";
import {
  type AnimPhase,
  type CheckReport,
  EASINGS,
  type MotionTemplate,
  type PropSpec,
  STARTER_SOURCE,
  TEXT_ANIMATIONS,
  checkCode,
  fontPreview,
  loadFonts,
  loadTemplates,
  readAnimation,
  resolvedProps,
  templateAsSource,
  templateItem,
  templateThumbnail,
  writeAnimation,
} from "./motion";
import "./motion.css";

const obj = (v: unknown): Fields => (v && typeof v === "object" && !Array.isArray(v) ? (v as Fields) : {});

function usePromise<T>(make: () => Promise<T>, deps: unknown[]): T | undefined {
  const [value, setValue] = useState<T | undefined>(undefined);
  useEffect(() => {
    let live = true;
    fire(
      make().then((v) => {
        if (live) setValue(v);
      }),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return value;
}

function Range({
  label,
  value,
  min,
  max,
  step,
  unit = "s",
  onCommit,
  testId,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string;
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
      <span className="slider-value">
        {shown.toFixed(step < 1 ? 1 : 0)}
        {unit}
      </span>
    </label>
  );
}

// ------------------------------------------------------------------------------------------------- text animation

/** Picks the entrance and exit animation of the selected text items, with a looping preview on each card. */
export function TextAnimationPicker({ targets, onEdit }: { targets: { id: string; textAnimation: unknown }[]; onEdit: (p: EditResult) => void }) {
  const [phase, setPhase] = useState<"in" | "out">("in");
  const current = readAnimation(targets[0]?.textAnimation);
  const active = current[phase];
  const write = (next: AnimPhase | null, summary: string) =>
    onEdit(
      updateItems(
        targets.map((t) => {
          const a = readAnimation(t.textAnimation);
          const v = writeAnimation({ ...a, [phase]: next });
          return v ? { id: t.id, set: { textAnimation: v } } : { id: t.id, unset: ["textAnimation"] };
        }),
        summary,
      ),
    );
  const pick = (preset: string) =>
    write(
      preset === "none"
        ? null
        : { preset, duration: active?.duration ?? (preset === "typewriter" ? 1.2 : 0.6), delay: active?.delay ?? 0, easing: active?.easing ?? (phase === "in" ? "ease-out" : "ease-in") },
      preset === "none" ? `no ${phase === "in" ? "entrance" : "exit"} animation` : `${phase === "in" ? "entrance" : "exit"} ${preset}`,
    );
  const groups = ["Basics", "Reveal", "Move", "Scale"] as const;
  return (
    <div className="anim-picker" data-testid="anim-picker">
      <div className="tabs anim-phase" role="tablist">
        {(["in", "out"] as const).map((p) => (
          <button key={p} role="tab" className={phase === p ? "on" : ""} aria-selected={phase === p} data-testid={`anim-phase-${p}`} onClick={() => setPhase(p)}>
            {p === "in" ? "In" : "Out"}
            {current[p] && <span className="anim-dot" />}
          </button>
        ))}
      </div>
      {groups.map((g) => (
        <div key={g}>
          <h6 className="anim-group">{g}</h6>
          <div className="anim-grid">
            {TEXT_ANIMATIONS.filter((a) => a.group === g).map((a) => {
              const on = (active?.preset ?? "none") === a.id;
              return (
                <button key={a.id} className={`anim-card${on ? " on" : ""}`} data-testid={`anim-${a.id}`} aria-pressed={on} onClick={() => pick(a.id)} title={a.label}>
                  <span className="anim-stage">
                    <span className={`anim-demo ta-${a.id} ${phase}`}>Aa</span>
                  </span>
                  <span className="anim-label">{a.label}</span>
                </button>
              );
            })}
          </div>
        </div>
      ))}
      {active && (
        <div className="anim-params">
          <Range label="Duration" value={active.duration} min={0.1} max={3} step={0.1} testId="anim-duration" onCommit={(v) => write({ ...active, duration: v }, "animation duration")} />
          <Range label={phase === "in" ? "Delay" : "Ends before"} value={active.delay} min={0} max={3} step={0.1} testId="anim-delay" onCommit={(v) => write({ ...active, delay: v }, "animation delay")} />
          <label className="select-row">
            <span>Easing</span>
            <select value={active.easing} data-testid="anim-easing" onChange={(e) => write({ ...active, easing: e.target.value }, "animation easing")}>
              {EASINGS.map((e) => (
                <option key={e.id} value={e.id}>
                  {e.label}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------------------------------------- fonts

export function FontPreview({ family, text, size = 20, className }: { family: string; text?: string; size?: number; className?: string }) {
  const url = usePromise(() => fontPreview(family, text ?? family, size), [family, text, size]);
  return url ? (
    <img className={`font-preview ${className ?? ""}`} src={url} alt={family} style={{ height: Math.round(size * 1.25) }} draggable={false} />
  ) : (
    <span className={`font-preview-text ${className ?? ""}`}>{text ?? family}</span>
  );
}

/** Font family dropdown: bundled families then installed ones, each previewed in its own face. */
export function FontPicker({ value, onChange }: { value: string; onChange: (family: string) => void }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const fonts = usePromise(loadFonts, []) ?? [];
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open]);
  const match = (f: FontFamily) => f.family.toLowerCase().includes(q.toLowerCase());
  const bundled = fonts.filter((f) => f.source === "bundled" && match(f));
  const system = fonts.filter((f) => f.source !== "bundled" && match(f)).slice(0, 80);
  const row = (f: FontFamily) => (
    <button
      key={f.family}
      role="option"
      aria-selected={f.family === value}
      className={`font-row${f.family === value ? " on" : ""}`}
      data-testid={`font-option-${f.family}`}
      onClick={() => {
        onChange(f.family);
        setOpen(false);
      }}
    >
      <FontPreview family={f.family} size={20} />
      <span className="font-meta">
        {f.category}
        {f.minWeight !== f.maxWeight ? ` · ${f.minWeight}–${f.maxWeight}` : ""}
      </span>
    </button>
  );
  return (
    <div className="font-picker" ref={box}>
      <button className="font-current" data-testid="insp-font" aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen(!open)}>
        <FontPreview family={value} size={18} />
        <span className="font-name">{value}</span>
        <Icon name="expand" size={12} />
      </button>
      {open && (
        <div className="font-menu" role="listbox" data-testid="font-menu">
          <label className="search">
            <Icon name="search" size={14} />
            <input autoFocus placeholder="Find a font…" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => (e.key === "Escape" ? setOpen(false) : e.stopPropagation())} />
          </label>
          {bundled.length > 0 && <h6 className="font-group">Included with the app</h6>}
          {bundled.map(row)}
          {system.length > 0 && <h6 className="font-group">Installed on this computer</h6>}
          {system.map(row)}
          {bundled.length + system.length === 0 && <p className="muted small">No font matches.</p>}
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------------------------------------- templates gallery

function TemplateCard({ t, disabled, onAdd }: { t: MotionTemplate; disabled: boolean; onAdd: () => void }) {
  const thumb = usePromise(() => templateThumbnail(t.id), [t.id]);
  return (
    <button className="preset-card motion-card" data-testid={`motion-template-${t.id}`} disabled={disabled} onClick={onAdd} title={t.description}>
      <span className="motion-thumb">{thumb ? <img src={thumb} alt="" draggable={false} /> : <Icon name="templates" size={20} />}</span>
      <span className="preset-label">{t.name}</span>
      <span className="muted small motion-desc">
        {t.duration}s · {t.fullFrame ? "full frame" : "overlay"}
      </span>
    </button>
  );
}

export interface MotionPanelProps {
  project: ProjectView_Serialize;
  playhead: number;
  onEdit: (p: EditResult) => void;
  onSelect: (item: ItemView_Serialize) => void;
}

/** Left-rail Motion panel: built-in templates and a blank composition, inserted as code items at the playhead. */
export function MotionPanel({ project, playhead, onEdit, onSelect }: MotionPanelProps) {
  const templates = usePromise(loadTemplates, []) ?? [];
  const track = firstTrack(project, "video", "t_text");
  const start = snapToFrame(playhead);
  const code = project.items.filter((i) => i.kind === "code");
  return (
    <>
      <p className="muted panel-note">Animated graphics drawn from code. Add one at the playhead, then edit its props or code in the Code tab.</p>
      <h5 className="panel-sub">Templates</h5>
      <div className="preset-grid motion-grid">
        {templates.map((t) => (
          <TemplateCard key={t.id} t={t} disabled={!track} onAdd={() => track && onEdit(addItems([templateItem(t, track, start)], `add ${t.name.toLowerCase()}`))} />
        ))}
      </div>
      <button
        className="btn wide"
        data-testid="motion-blank"
        disabled={!track}
        onClick={() =>
          track &&
          onEdit(
            addItems(
              [{ trackId: track, type: "code", startTime: start, endTime: snapToFrame(start + 4), name: "Composition", codeComposition: { source: STARTER_SOURCE, props: { text: "Hello" } } }],
              "add composition",
            ),
          )
        }
      >
        <Icon name="plus" size={15} /> Blank composition (code)
      </button>
      <h5 className="panel-sub">In this project</h5>
      {code.length === 0 && <p className="muted panel-note">No motion graphics yet.</p>}
      {code.map((c) => (
        <button key={c.id} className="list-row" onClick={() => onSelect(c)}>
          <span className="muted small">{(c.start ?? 0).toFixed(1)}s</span> {c.name}
        </button>
      ))}
    </>
  );
}

// ------------------------------------------------------------------------------------------------- Code tab

function PropField({ spec, value, images, onCommit }: { spec: PropSpec; value: unknown; images: { id: string; name: string }[]; onCommit: (v: unknown) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const text =
    typeof value === "string"
      ? value
      : typeof value === "number" || typeof value === "boolean"
        ? String(value)
        : value == null
          ? ""
          : JSON.stringify(value);
  const id = `prop-${spec.name}`;
  switch (spec.kind) {
    case "color":
      return (
        <label className="color-field">
          <span>{spec.label}</span>
          <input type="color" data-testid={id} value={/^#[0-9a-f]{6}/i.test(text) ? text.slice(0, 7) : "#ffffff"} onChange={(e) => onCommit(e.target.value)} />
          <code>{text}</code>
        </label>
      );
    case "boolean":
      return (
        <button className={`toggle-row${value === true ? " on" : ""}`} role="switch" aria-checked={value === true} data-testid={id} onClick={() => onCommit(value !== true)}>
          <span>{spec.label}</span>
          <span className="switch" />
        </button>
      );
    case "select":
      return (
        <label className="select-row">
          <span>{spec.label}</span>
          <select value={text} data-testid={id} onChange={(e) => onCommit(e.target.value)}>
            {(spec.options ?? []).map((o) => (
              <option key={o}>{o}</option>
            ))}
          </select>
        </label>
      );
    case "image":
      return (
        <label className="select-row">
          <span>{spec.label}</span>
          <select value={text} data-testid={id} onChange={(e) => onCommit(e.target.value || null)}>
            <option value="">None</option>
            {images.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        </label>
      );
    case "number":
      return (
        <label className="num-field">
          <span>{spec.label}</span>
          <input
            inputMode="decimal"
            data-testid={id}
            value={draft ?? text}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => {
              const v = Number(draft);
              if (draft !== null && Number.isFinite(v)) onCommit(Math.min(spec.max ?? Infinity, Math.max(spec.min ?? -Infinity, v)));
              setDraft(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              e.stopPropagation();
            }}
          />
        </label>
      );
    default: {
      const commit = () => {
        if (draft !== null && draft !== text) onCommit(draft);
        setDraft(null);
      };
      return (
        <label className="prop-text">
          <span>{spec.label}</span>
          {spec.kind === "multiline" ? (
            <textarea className="field-text" rows={3} data-testid={id} value={draft ?? text} onChange={(e) => setDraft(e.target.value)} onBlur={commit} onKeyDown={(e) => e.stopPropagation()} />
          ) : (
            <input
              className="field-text"
              data-testid={id}
              value={draft ?? text}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commit}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                e.stopPropagation();
              }}
            />
          )}
        </label>
      );
    }
  }
}

function Diagnostics({ report, checking, onJump }: { report: CheckReport | null; checking: boolean; onJump?: (line: number, column: number) => void }) {
  if (!report) return <p className="muted small">{checking ? "Checking…" : ""}</p>;
  const errors = report.diagnostics.filter((d) => d.severity === "error").length;
  const warnings = report.diagnostics.length - errors;
  return (
    <div className="code-diags" data-testid="code-diagnostics">
      <p className={`diag-summary ${errors ? "bad" : warnings ? "warn" : "good"}`} data-testid="code-status">
        {errors ? `${errors} error${errors > 1 ? "s" : ""}` : warnings ? `${warnings} warning${warnings > 1 ? "s" : ""}` : "No problems"}
        {report.frameMs > 0 && (
          <span className="muted">
            {" "}
            · slowest frame {report.frameMs.toFixed(0)} ms at {report.width}×{report.height}
          </span>
        )}
        {checking && <span className="muted"> · checking…</span>}
      </p>
      {report.diagnostics.map((d, i) => (
        <button
          key={i}
          className={`diag ${d.severity}`}
          data-testid="code-diagnostic"
          disabled={!onJump || d.line == null}
          onClick={() => d.line != null && onJump?.(d.line, d.column ?? 1)}
        >
          <span className="diag-sev">{d.severity === "error" ? "✕" : "!"}</span>
          {d.line != null && <span className="diag-pos">{`${d.line}:${d.column ?? 1}`}</span>}
          <span className="diag-msg">{d.message}</span>
        </button>
      ))}
      {report.logs.length > 0 && (
        <div className="diag-logs">
          <h6>Console</h6>
          {report.logs.map((l, i) => (
            <code key={i}>{l}</code>
          ))}
        </div>
      )}
    </div>
  );
}

/** Offset of a 1-based line and column in `text`. */
function offsetOf(text: string, line: number, column: number): number {
  let off = 0;
  for (let l = 1; l < line; l++) {
    const nl = text.indexOf("\n", off);
    if (nl < 0) break;
    off = nl + 1;
  }
  return Math.min(text.length, off + Math.max(0, column - 1));
}

/** Inspector tab of a code item: template props or source, with live diagnostics from the real renderer. */
export function CodeTab({ project, item, detail, set }: { project: ProjectView_Serialize; item: ItemView_Serialize; detail: Fields; set: (f: Fields, summary?: string) => void }) {
  const comp = obj(detail.codeComposition);
  const templates = usePromise(loadTemplates, []) ?? [];
  const template = typeof comp.template === "string" ? templates.find((t) => t.id === comp.template) : undefined;
  const source = typeof comp.source === "string" ? comp.source : null;
  const [draft, setDraft] = useState<string | null>(null);
  const [report, setReport] = useState<CheckReport | null>(null);
  const [checking, setChecking] = useState(false);
  const editor = useRef<HTMLTextAreaElement>(null);
  const duration = Math.max(0.1, (item.end ?? 0) - (item.start ?? 0));
  const key = JSON.stringify(comp);
  const images = useMemo(() => project.media.filter((m) => m.kind === "image").map((m) => ({ id: m.id, name: m.name })), [project.media]);

  const run = (c: Fields) => {
    setChecking(true);
    fire(
      checkCode(c, duration)
        .then(setReport)
        .finally(() => setChecking(false)),
    );
  };
  useEffect(() => {
    if (!key || key === "{}") return;
    const t = setTimeout(() => run(JSON.parse(key) as Fields), 150);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, duration]);

  const setComp = (next: Fields, summary: string) => set({ codeComposition: next }, summary);
  const jump = (line: number, column: number) => {
    const ta = editor.current;
    if (!ta) return;
    const off = offsetOf(ta.value, line, column);
    ta.focus();
    ta.setSelectionRange(off, off + 1);
  };

  if (!comp.template && source === null) return <p className="muted">This code item has no composition.</p>;
  return (
    <>
      {template && (
        <section className="insp-section">
          <header>
            <h4>{template.name}</h4>
            <button className="link" data-testid="code-edit-source" onClick={() => setComp(templateAsSource(template, obj(comp.props)), "edit template as code")}>
              Edit as code
            </button>
          </header>
          <p className="muted small">{template.description}</p>
          <div className="prop-form" data-testid="template-props">
            {template.props.map((p) => (
              <PropField
                key={p.name}
                spec={p}
                images={images}
                value={resolvedProps(template, obj(comp.props))[p.name]}
                onCommit={(v) => setComp({ ...comp, props: { ...obj(comp.props), [p.name]: v } }, `${template.name.toLowerCase()} ${p.label.toLowerCase()}`)}
              />
            ))}
          </div>
        </section>
      )}
      {source !== null && (
        <section className="insp-section">
          <header>
            <h4>Source</h4>
            <span className="code-actions">
              <button className="link" disabled={draft === null} onClick={() => setDraft(null)}>
                Revert
              </button>
              <button className="link" data-testid="code-check-draft" onClick={() => run({ ...comp, source: draft ?? source })}>
                Check
              </button>
              <button className="btn small" data-testid="code-apply" disabled={draft === null || draft === source} onClick={() => draft !== null && (setComp({ ...comp, source: draft }, "edit composition code"), setDraft(null))}>
                Apply
              </button>
            </span>
          </header>
          <textarea
            ref={editor}
            className="code-editor"
            spellCheck={false}
            data-testid="code-source"
            rows={18}
            value={draft ?? source}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Tab") {
                e.preventDefault();
                const ta = e.currentTarget;
                const { selectionStart: a, selectionEnd: b, value } = ta;
                const next = value.slice(0, a) + "  " + value.slice(b);
                setDraft(next);
                requestAnimationFrame(() => ta.setSelectionRange(a + 2, a + 2));
              }
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && draft !== null) {
                setComp({ ...comp, source: draft }, "edit composition code");
                setDraft(null);
              }
            }}
          />
          <PropsJson value={obj(comp.props)} onCommit={(props) => setComp({ ...comp, props }, "composition props")} />
        </section>
      )}
      <section className="insp-section">
        <header>
          <h4>Check</h4>
        </header>
        <Diagnostics report={report} checking={checking} onJump={source !== null ? jump : undefined} />
      </section>
    </>
  );
}

function PropsJson({ value, onCommit }: { value: Fields; onCommit: (v: Fields) => void }) {
  const text = JSON.stringify(value, null, 1);
  const [draft, setDraft] = useState<string | null>(null);
  const [bad, setBad] = useState(false);
  return (
    <label className="prop-text">
      <span>Props (JSON)</span>
      <textarea
        className={`field-text code-small${bad ? " bad" : ""}`}
        rows={3}
        spellCheck={false}
        data-testid="code-props"
        value={draft ?? text}
        onChange={(e) => {
          setDraft(e.target.value);
          setBad(false);
        }}
        onBlur={() => {
          if (draft === null) return;
          try {
            const v = JSON.parse(draft) as unknown;
            if (v && typeof v === "object" && !Array.isArray(v)) {
              onCommit(v as Fields);
              setDraft(null);
            } else setBad(true);
          } catch {
            setBad(true);
          }
        }}
        onKeyDown={(e) => e.stopPropagation()}
      />
    </label>
  );
}
