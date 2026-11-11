// Editing helpers shared by the inspector, the panels and the timeline. Every call is one revision in the project
// document (undoable, durable, mirrored in timeline.tsx).

import type { AppError, ItemView_Serialize, ProjectView_Serialize } from "./bindings";
import { commands } from "./bridge";

export type EditResult = Promise<{ status: "ok"; data: ProjectView_Serialize } | { status: "error"; error: AppError }>;
export type Fields = Record<string, unknown>;

export const updateItem = (id: string, set: Fields, unset: string[] = [], summary: string | null = null): EditResult =>
  commands.updateItem(id, JSON.stringify(set), unset, summary);

export const updateItems = (edits: { id: string; set?: Fields; unset?: string[] }[], summary: string | null = null): EditResult =>
  commands.updateItems(JSON.stringify(edits), summary);

export const addItems = (items: Fields[], summary: string | null = null): EditResult => commands.addItems(JSON.stringify(items), summary);

/** Full item JSON (document field names) for the inspector. */
export async function itemDetail(id: string): Promise<Fields | null> {
  const r = await commands.itemDetail(id);
  return r.status === "ok" ? (JSON.parse(r.data) as Fields) : null;
}

// ------------------------------------------------------------------------------------------------- tracks

export function firstTrack(p: ProjectView_Serialize, kind: string, preferId?: string): string | null {
  if (preferId && p.tracks.some((t) => t.id === preferId)) return preferId;
  return p.tracks.find((t) => t.kind === kind)?.id ?? null;
}

/** Kinds of items a track accepts. */
export function trackAccepts(trackKind: string, itemKind: string): boolean {
  if (trackKind === "audio") return itemKind === "audio";
  if (trackKind === "caption") return itemKind === "caption";
  if (trackKind === "adjustment") return itemKind === "adjustment" || itemKind === "shader";
  return itemKind !== "audio" && itemKind !== "caption";
}

// ------------------------------------------------------------------------------------------------- presets

export interface TextPreset {
  id: string;
  label: string;
  /** Preview text on the card. */
  sample: string;
  duration: number;
  fields: Fields;
  /** CSS for the card preview. */
  style: React.CSSProperties;
}

export const TEXT_PRESETS: TextPreset[] = [
  {
    id: "headline",
    label: "Headline",
    sample: "Big Story",
    duration: 4,
    fields: { textContent: "Big Story", fontFamily: "Inter", fontSize: 140, fontWeight: 800, color: "#ffffff", textAlign: "center" },
    style: { fontWeight: 800, fontSize: 28, lineHeight: 1 },
  },
  {
    id: "serif",
    label: "Film title",
    sample: "Northern Light",
    duration: 5,
    fields: { textContent: "Northern Light", fontFamily: "Georgia", fontSize: 110, fontWeight: 400, color: "#f4ead7", textAlign: "center", letterSpacing: 2 },
    style: { fontFamily: "Georgia, serif", fontSize: 24, color: "#f4ead7", lineHeight: 1.05 },
  },
  {
    id: "outline",
    label: "Outline",
    sample: "LOUD",
    duration: 3,
    fields: { textContent: "LOUD", fontFamily: "Inter", fontSize: 180, fontWeight: 900, color: "#00000000", strokeColor: "#ffffff", strokeWidth: 4, textAlign: "center" },
    style: { fontWeight: 900, fontSize: 34, color: "transparent", WebkitTextStroke: "1.5px #fff" },
  },
  {
    id: "boxed",
    label: "Boxed",
    sample: "On Location",
    duration: 4,
    fields: { textContent: "On Location", fontFamily: "Inter", fontSize: 72, fontWeight: 600, color: "#111111", backgroundColor: "#ffd84d", backgroundOpacity: 1, backgroundPadding: 24, textAlign: "center" },
    style: { fontWeight: 600, fontSize: 16, color: "#111", background: "#ffd84d", padding: "4px 8px" },
  },
  {
    id: "lower-third",
    label: "Lower third",
    sample: "Ana Ruiz · Director",
    duration: 5,
    fields: {
      textContent: "Ana Ruiz\nDirector",
      fontFamily: "Inter",
      fontSize: 54,
      fontWeight: 600,
      color: "#ffffff",
      textAlign: "left",
      backgroundColor: "#000000",
      backgroundOpacity: 0.55,
      backgroundPadding: 18,
      transform: { x: -560, y: 330 },
    },
    style: { fontSize: 13, fontWeight: 600, background: "rgba(0,0,0,.6)", padding: "4px 8px", alignSelf: "flex-end", justifySelf: "start" },
  },
  {
    id: "subtitle",
    label: "Subtitle",
    sample: "and then we set off",
    duration: 3,
    fields: { textContent: "and then we set off", fontFamily: "Inter", fontSize: 48, fontWeight: 500, color: "#ffffff", textAlign: "center", shadowColor: "#000000", shadowBlur: 8, transform: { y: 380 } },
    style: { fontSize: 13, fontWeight: 500, textShadow: "0 1px 4px #000", alignSelf: "flex-end" },
  },
  {
    id: "number",
    label: "Big number",
    sample: "87%",
    duration: 3,
    fields: { textContent: "87%", fontFamily: "Inter", fontSize: 260, fontWeight: 900, color: "#7cf0b0", textAlign: "center" },
    style: { fontWeight: 900, fontSize: 38, color: "#7cf0b0" },
  },
  {
    id: "quote",
    label: "Quote",
    sample: "“Make it simple.”",
    duration: 5,
    fields: { textContent: "“Make it simple.”", fontFamily: "Georgia", fontStyle: "italic", fontSize: 84, color: "#ffffff", textAlign: "center", lineHeight: 1.2 },
    style: { fontFamily: "Georgia, serif", fontStyle: "italic", fontSize: 18 },
  },
];

export interface LookPreset {
  id: string;
  label: string;
  grading: Fields;
  /** CSS gradient for the card. */
  swatch: string;
}

export const LOOK_PRESETS: LookPreset[] = [
  { id: "warm", label: "Warm", grading: { temperature: 25, tint: 4, saturation: 8 }, swatch: "linear-gradient(135deg,#f6b46b,#c9653b)" },
  { id: "cool", label: "Cool", grading: { temperature: -25, tint: -3, saturation: 4 }, swatch: "linear-gradient(135deg,#7fb6e8,#36598f)" },
  { id: "punchy", label: "Punchy", grading: { contrast: 25, saturation: 25, vibrance: 15 }, swatch: "linear-gradient(135deg,#ff4f81,#4f6bff)" },
  { id: "soft", label: "Soft", grading: { contrast: -15, highlights: -10, shadows: 12 }, swatch: "linear-gradient(135deg,#e8dde3,#9d8f99)" },
  { id: "mono", label: "Mono", grading: { saturation: -100, contrast: 10 }, swatch: "linear-gradient(135deg,#eeeeee,#333333)" },
  { id: "faded", label: "Faded film", grading: { contrast: -20, saturation: -25, shadows: 18, temperature: 8 }, swatch: "linear-gradient(135deg,#d8c9a6,#6e6a5a)" },
  { id: "night", label: "Night", grading: { exposure: -0.6, temperature: -30, saturation: -10 }, swatch: "linear-gradient(135deg,#2b3f6b,#0b1020)" },
  { id: "bright", label: "Bright", grading: { exposure: 0.4, shadows: 15, highlights: -5 }, swatch: "linear-gradient(135deg,#ffffff,#ffe9a8)" },
];

export interface TransitionPreset {
  id: string;
  label: string;
  type: string | null;
}

export const TRANSITIONS: TransitionPreset[] = [
  { id: "none", label: "None", type: null },
  { id: "fade", label: "Fade", type: "fade" },
  { id: "crossfade", label: "Cross-dissolve", type: "crossfade" },
  { id: "dip-black", label: "Dip to black", type: "dip-to-black" },
  { id: "dip-white", label: "Dip to white", type: "dip-to-white" },
  { id: "slide-left", label: "Slide left", type: "slide-left" },
  { id: "wipe", label: "Wipe", type: "wipe" },
  { id: "zoom", label: "Zoom", type: "zoom" },
];

export const ASPECTS = [
  { id: "16:9", label: "Landscape", hint: "YouTube, TV" },
  { id: "9:16", label: "Vertical", hint: "Reels, Shorts, TikTok" },
  { id: "1:1", label: "Square", hint: "Feed posts" },
  { id: "4:5", label: "Portrait", hint: "Feed, tall" },
  { id: "4:3", label: "Classic", hint: "Presentations" },
  { id: "3:4", label: "Classic tall", hint: "Print, tablets" },
];

/** Adjacent clip on the same track that starts where `item` ends (within 50 ms), if any. */
export function nextAdjacent(p: ProjectView_Serialize, item: ItemView_Serialize): ItemView_Serialize | undefined {
  return p.items.find((i) => i.trackId === item.trackId && i.id !== item.id && Math.abs((i.start ?? 0) - (item.end ?? 0)) < 0.05);
}

/** Project frame rate used for placing items (the timeline renders at 30 fps). */
export const TIMELINE_FPS = 30;

/** Snaps a time in seconds to the nearest frame of the timeline grid, at microsecond precision. */
export function snapToFrame(t: number, fps = TIMELINE_FPS): number {
  const frames = Math.round(Math.max(0, t) * fps);
  return Math.round((frames / fps) * 1e6) / 1e6;
}
