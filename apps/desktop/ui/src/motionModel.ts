// Motion graphics and fonts: template schemas, text-animation presets, font list and previews, code checks.
// Data comes from the native core (ove-graphics) through the bridge; results are cached for the session.

import type { FontFamily } from "./bindings";
import { commands } from "./bridge";
import type { Fields } from "./edit";

// ------------------------------------------------------------------------------------------------- templates

export type PropKind = "text" | "multiline" | "color" | "number" | "boolean" | "select" | "image";

export interface PropSpec {
  name: string;
  label: string;
  kind: PropKind;
  default: unknown;
  min?: number;
  max?: number;
  step?: number;
  options?: string[];
}

export interface MotionTemplate {
  id: string;
  name: string;
  description: string;
  duration: number;
  fullFrame: boolean;
  props: PropSpec[];
  source: string;
}

let templatesCache: Promise<MotionTemplate[]> | null = null;

export function loadTemplates(): Promise<MotionTemplate[]> {
  templatesCache ??= commands.listTemplates().then((r) => (r.status === "ok" ? (JSON.parse(r.data) as MotionTemplate[]) : []));
  return templatesCache;
}

const thumbs = new Map<string, Promise<string | null>>();
export function templateThumbnail(id: string): Promise<string | null> {
  if (!thumbs.has(id)) thumbs.set(id, commands.templateThumbnail(id).then((r) => (r.status === "ok" ? r.data : null)));
  return thumbs.get(id)!;
}

/** Template defaults overlaid with the item's props. */
export function resolvedProps(t: MotionTemplate, props: Fields): Fields {
  const out: Fields = {};
  for (const p of t.props) out[p.name] = props[p.name] !== undefined ? props[p.name] : p.default;
  return out;
}

/** The fields of a new code item for a template, at `start` on `trackId`. */
export function templateItem(t: MotionTemplate, trackId: string, start: number): Fields {
  return {
    trackId,
    type: "code",
    startTime: start,
    endTime: start + t.duration,
    name: t.name,
    codeComposition: { template: t.id, props: {} },
  };
}

/** A template composition turned into editable source, keeping the current prop values. */
export function templateAsSource(t: MotionTemplate, props: Fields): Fields {
  const all = resolvedProps(t, props);
  const assets: Fields = {};
  const plain: Fields = {};
  for (const p of t.props) {
    if (p.kind === "image") {
      if (typeof all[p.name] === "string" && all[p.name]) assets[p.name] = all[p.name];
    } else plain[p.name] = all[p.name];
  }
  return { source: t.source, props: plain, ...(Object.keys(assets).length ? { assets } : {}) };
}

export const STARTER_SOURCE = `// A composition is a pure function of time. See the Code tab for what is supported.
export default function Composition({ time, duration, props }) {
  const enter = easing.out(clamp(time / 0.6));
  const leave = easing.in(clamp((time - (duration - 0.5)) / 0.5));
  return (
    <Fill style={{ alignItems: "center", justifyContent: "center" }}>
      <div
        style={{
          padding: "24px 48px",
          borderRadius: 24,
          backgroundColor: "rgba(10, 12, 20, 0.7)",
          fontSize: 96,
          fontWeight: 800,
          opacity: enter * (1 - leave),
          transform: \`translateY(\${(1 - enter) * 40}px)\`,
        }}
      >
        {props.text ?? "Hello"}
      </div>
    </Fill>
  );
}
`;

// ------------------------------------------------------------------------------------------------- checks

export interface Diagnostic {
  severity: "error" | "warning";
  code: string;
  message: string;
  line: number | null;
  column: number | null;
}

export interface CheckReport {
  ok: boolean;
  diagnostics: Diagnostic[];
  logs: string[];
  frameMs: number;
  width: number;
  height: number;
}

export async function checkCode(composition: Fields, duration: number): Promise<CheckReport> {
  const r = await commands.checkCode(JSON.stringify(composition), duration);
  if (r.status === "error") {
    return { ok: false, diagnostics: [{ severity: "error", code: "check", message: r.error.message, line: null, column: null }], logs: [], frameMs: 0, width: 0, height: 0 };
  }
  return JSON.parse(r.data) as CheckReport;
}

// ------------------------------------------------------------------------------------------------- text animation

export interface AnimPreset {
  id: string;
  label: string;
  group: "Basics" | "Reveal" | "Move" | "Scale";
}

/** The presets of docs/spec/presets.md, rendered by ove-render's text_anim.rs. */
export const TEXT_ANIMATIONS: AnimPreset[] = [
  { id: "none", label: "None", group: "Basics" },
  { id: "fade", label: "Fade", group: "Basics" },
  { id: "blur", label: "Blur", group: "Basics" },
  { id: "typewriter", label: "Typewriter", group: "Reveal" },
  { id: "glitch", label: "Glitch", group: "Reveal" },
  { id: "rise", label: "Rise", group: "Move" },
  { id: "drop", label: "Drop", group: "Move" },
  { id: "slide-left", label: "Slide left", group: "Move" },
  { id: "slide-right", label: "Slide right", group: "Move" },
  { id: "settle-up", label: "Settle up", group: "Move" },
  { id: "settle-down", label: "Settle down", group: "Move" },
  { id: "zoom-in", label: "Zoom in", group: "Scale" },
  { id: "zoom-out", label: "Zoom out", group: "Scale" },
  { id: "pop", label: "Pop", group: "Scale" },
  { id: "focus-pop", label: "Focus pop", group: "Scale" },
  { id: "rotate-in", label: "Rotate in", group: "Scale" },
];

export const EASINGS = [
  { id: "ease-out", label: "Ease out" },
  { id: "ease-in", label: "Ease in" },
  { id: "ease-in-out", label: "Ease in-out" },
  { id: "linear", label: "Linear" },
  { id: "back", label: "Overshoot" },
  { id: "elastic", label: "Elastic" },
];

export interface AnimPhase {
  preset: string;
  duration: number;
  delay: number;
  easing: string;
}

const phaseOf = (v: unknown, easing: string): AnimPhase | null => {
  if (typeof v === "string") return v === "none" ? null : { preset: v, duration: 0.6, delay: 0, easing };
  if (!v || typeof v !== "object") return null;
  const o = v as Fields;
  if (typeof o.preset !== "string" || o.preset === "none") return null;
  return {
    preset: o.preset,
    duration: typeof o.duration === "number" ? o.duration : o.preset === "typewriter" ? 1.2 : 0.6,
    delay: typeof o.delay === "number" ? o.delay : 0,
    easing: typeof o.easing === "string" ? o.easing : easing,
  };
};

/** Entrance and exit of an item's `textAnimation`. */
export function readAnimation(v: unknown): { in: AnimPhase | null; out: AnimPhase | null } {
  const o = v && typeof v === "object" ? (v as Fields) : {};
  return { in: phaseOf(v, "ease-out"), out: phaseOf(o.out, "ease-in") };
}

/** The `textAnimation` value for an entrance and exit, or null to remove it. */
export function writeAnimation(a: { in: AnimPhase | null; out: AnimPhase | null }): Fields | null {
  if (!a.in && !a.out) return null;
  const v: Fields = a.in ? { ...a.in } : { preset: "none" };
  if (a.out) v.out = { ...a.out };
  return v;
}

// ------------------------------------------------------------------------------------------------- fonts

let fontsCache: Promise<FontFamily[]> | null = null;
export function loadFonts(): Promise<FontFamily[]> {
  fontsCache ??= commands.listFonts().then((r) => (r.status === "ok" ? r.data : []));
  return fontsCache;
}

const previews = new Map<string, Promise<string | null>>();
/** A PNG data URL of `text` in `family` (rendered by the same text engine as the timeline). */
export function fontPreview(family: string, text = family, size = 22): Promise<string | null> {
  const key = `${family}\u0001${text}\u0001${size}`;
  if (!previews.has(key)) previews.set(key, commands.fontPreview(family, text, size).then((r) => (r.status === "ok" ? r.data : null)));
  return previews.get(key)!;
}
