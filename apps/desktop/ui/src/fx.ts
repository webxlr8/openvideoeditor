// Effects, transitions, layouts and looks: the catalogue (from the core, so the UI never drifts from the renderer's
// parameter contracts) and small helpers to read and write the item fields that use them.

import { useEffect, useState } from "react";
import { commands } from "./bridge";
import type { Fields } from "./edit";

export type ParamKind =
  | { kind: "color" }
  | { kind: "percent"; min: number; max: number }
  | { kind: "number"; min: number; max: number }
  | { kind: "point" }
  | { kind: "toggle" };

export interface ParamSpec {
  name: string;
  kind: ParamKind;
  default: number | number[];
}

export interface EffectSpec {
  id: string;
  label: string;
  group: "stylize" | "distort" | "background";
  params: ParamSpec[];
  animated: boolean;
}

export interface TransitionSpec {
  id: string;
  label: string;
  directional: boolean;
  audioOnly: boolean;
}

export interface LayoutSpec {
  id: string;
  label: string;
  group: string;
  slots: [number, number, number, number][];
}

export interface FxCatalog {
  effects: EffectSpec[];
  transitions: TransitionSpec[];
  easings: string[];
  directions: string[];
  layouts: LayoutSpec[];
  luts: { id: string; label: string }[];
}

let catalog: Promise<FxCatalog> | null = null;

export function loadCatalog(): Promise<FxCatalog> {
  catalog ??= commands.fxCatalog().then((r) => {
    if (r.status !== "ok") throw new Error(r.error.message);
    return JSON.parse(r.data) as FxCatalog;
  });
  catalog.catch(() => (catalog = null));
  return catalog;
}

export function useCatalog(): FxCatalog | null {
  const [c, setC] = useState<FxCatalog | null>(null);
  useEffect(() => {
    let live = true;
    void loadCatalog().then((x) => live && setC(x));
    return () => {
      live = false;
    };
  }, []);
  return c;
}

const previews = new Map<string, Promise<string>>();

/** Thumbnail data URL of an effect at its defaults ("effect") or a look ("lut"). */
export function usePreview(kind: "effect" | "lut", id: string): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const key = `${kind}:${id}`;
    let p = previews.get(key);
    if (!p) {
      p = commands.fxPreview(kind, id).then((r) => (r.status === "ok" ? r.data : ""));
      previews.set(key, p);
    }
    void p.then((u) => live && setUrl(u || null));
    return () => {
      live = false;
    };
  }, [kind, id]);
  return url;
}

export const GROUP_LABEL: Record<EffectSpec["group"], string> = { stylize: "Stylize", distort: "Distort", background: "Backgrounds" };

// ---------------------------------------------------------------------------------------------------------------
// Values

/** Default value of a parameter as stored in a document (colours [r,g,b] 0..1, points [x,y], toggles booleans). */
export function defaultValue(p: ParamSpec): unknown {
  if (p.kind.kind === "toggle") return p.default === 1;
  return p.default;
}

export function paramValue(values: Fields, p: ParamSpec): unknown {
  return values[p.name] ?? defaultValue(p);
}

export const rgbToHex = (c: unknown): string => {
  const a = Array.isArray(c) ? c : [1, 1, 1];
  return `#${a
    .slice(0, 3)
    .map((v) => Math.round(Math.min(1, Math.max(0, Number(v) || 0)) * 255).toString(16).padStart(2, "0"))
    .join("")}`;
};

export const hexToRgb = (h: string): number[] => {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(h);
  return m ? [1, 2, 3].map((i) => Math.round((parseInt(m[i], 16) / 255) * 1000) / 1000) : [1, 1, 1];
};

export interface EffectEntry extends Fields {
  id: string;
  type: string;
}

export function effectStack(fields: Fields): EffectEntry[] {
  const e = fields.effects;
  return Array.isArray(e) ? (e.filter((x) => x && typeof x === "object" && typeof (x as Fields).type === "string") as EffectEntry[]) : [];
}

let nextFx = 0;
export const newEffectId = () => `fx_${Date.now().toString(36)}${(nextFx++).toString(36)}`;

/** A new stack entry with the contract defaults written out (so the document says exactly what renders). */
export function newEffect(spec: EffectSpec): EffectEntry {
  const e: EffectEntry = { id: newEffectId(), type: spec.id };
  for (const p of spec.params) e[p.name] = defaultValue(p);
  return e;
}

/** Splits `wipe-left` into its kind and direction (the renderer accepts both spellings). */
export function splitTransition(type: string, direction?: unknown): { base: string; direction: string | null } {
  const m = /^(wipe|push|fly)-(left|right|up|down)$/.exec(type);
  if (m) return { base: m[1], direction: m[2] };
  return { base: type, direction: typeof direction === "string" ? direction : null };
}
