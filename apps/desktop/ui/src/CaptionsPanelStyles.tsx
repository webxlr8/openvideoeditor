// Caption styles in the UI: the style list from the core (the same presets the renderer draws), a live CSS preview
// of each, and transcript helpers shared by the Captions panel and the inspector.

import { type CSSProperties, useEffect, useState } from "react";
import { commands } from "./bridge";
import { fire, reportError } from "./notify";

export interface CaptionMark {
  kind: "none" | "color" | "box" | "ring" | "underline";
  color?: string;
  text?: string;
}

export interface CaptionPreset {
  id: string;
  label: string;
  family: string;
  fontSize: number;
  fontWeight: number;
  textColor: string;
  highlightColor: string | null;
  mark: CaptionMark;
  position: "top" | "center" | "bottom";
  align: "left" | "center";
  uppercase: boolean;
  lowercase: boolean;
  letterSpacing: number;
  animation: string;
  stroke: { color: string; width: number } | null;
  shadow: { color: string; x: number; y: number } | null;
  panel: { color: string; perWord: boolean; radius: number } | null;
  composition: string;
}

export const DEFAULT_CAPTION_PRESET = "butter-serif";

let cached: Promise<CaptionPreset[]> | null = null;

/** The caption styles, loaded once from the core. */
export function loadCaptionPresets(): Promise<CaptionPreset[]> {
  cached ??= commands
    .captionPresets()
    .then((json) => JSON.parse(json) as CaptionPreset[])
    .catch((e: unknown) => {
      cached = null;
      reportError(e);
      return [];
    });
  return cached;
}

export function useCaptionPresets(): CaptionPreset[] {
  const [list, setList] = useState<CaptionPreset[]>([]);
  useEffect(() => {
    let live = true;
    fire(
      loadCaptionPresets().then((l) => {
        if (live) setList(l);
      }),
    );
    return () => {
      live = false;
    };
  }, []);
  return list;
}

const ANIMATION_LABEL: Record<string, string> = {
  word: "Word highlight",
  "word-pop": "Word pop",
  pulse: "Pulse",
  reveal: "Reveal",
  stacked: "Stacked",
  spotlight: "Spotlight",
};

export const animationLabel = (id: string) => ANIMATION_LABEL[id] ?? id;

const FAMILY: Record<string, string> = {
  serif: "Georgia, 'Times New Roman', serif",
  script: "'Brush Script MT', 'Segoe Script', cursive",
  condensed: "'Arial Narrow', 'Roboto Condensed', Inter, sans-serif",
  display: "Inter, system-ui, sans-serif",
  sans: "Inter, system-ui, sans-serif",
};

/** A sample of the style: "Make every video" with the third word marked as the spoken one. */
export function CaptionSample({ preset, words = ["Make", "every", "video"], active = 2 }: { preset: CaptionPreset; words?: string[]; active?: number }) {
  const p = preset;
  const em = Math.max(11, Math.min(22, p.fontSize / 4.6));
  const shadows: string[] = [];
  if (p.stroke) {
    const w = Math.max(1, p.stroke.width * em);
    for (const [x, y] of [[w, 0], [-w, 0], [0, w], [0, -w], [w, w], [-w, -w], [w, -w], [-w, w]]) shadows.push(`${x}px ${y}px 0 ${p.stroke.color}`);
  }
  if (p.shadow) shadows.push(`${p.shadow.x * em}px ${Math.max(1, p.shadow.y * em)}px ${em * 0.15}px ${p.shadow.color}`);
  const line: CSSProperties = {
    fontFamily: FAMILY[p.family] ?? FAMILY.sans,
    fontSize: em,
    fontWeight: p.fontWeight,
    color: p.textColor,
    letterSpacing: `${p.letterSpacing}em`,
    textTransform: p.uppercase ? "uppercase" : p.lowercase ? "lowercase" : "none",
    textShadow: shadows.join(", ") || undefined,
    lineHeight: 1.1,
    textAlign: p.align,
    fontStyle: p.id.includes("italic") ? "italic" : undefined,
    display: "flex",
    flexWrap: "wrap",
    flexDirection: p.animation === "stacked" ? "column" : "row",
    alignItems: p.align === "left" ? "flex-start" : "center",
    justifyContent: "center",
    gap: p.animation === "stacked" ? 0 : "0.28em",
    padding: p.panel && !p.panel.perWord ? "0.15em 0.4em" : undefined,
    background: p.panel && !p.panel.perWord ? p.panel.color : undefined,
    boxShadow: p.id === "neo-brutal" ? "0.12em 0.12em 0 #111" : undefined,
  };
  return (
    <span className="caption-sample-line" style={line} data-testid={`caption-sample-${p.id}`}>
      {words.map((w, i) => {
        const on = i === active;
        const st: CSSProperties = {};
        if (p.panel?.perWord) {
          Object.assign(st, { background: on && p.mark.color ? p.mark.color : i % 3 === 1 ? "#e8c9a0" : p.panel.color, padding: "0 0.18em", transform: `rotate(${i % 2 ? 2 : -2.5}deg)`, margin: "0.04em 0" });
          if (on && p.mark.text) st.color = p.mark.text;
        } else if (on) {
          if (p.mark.kind === "color") st.color = p.mark.color;
          if (p.mark.kind === "box") Object.assign(st, { background: p.mark.color, color: p.mark.text, padding: "0 0.18em", borderRadius: "0.18em", textShadow: "none" });
          if (p.mark.kind === "ring") Object.assign(st, { outline: `0.08em solid ${p.mark.color}`, borderRadius: "50%", padding: "0 0.3em" });
          if (p.mark.kind === "underline") Object.assign(st, { textDecoration: `underline ${p.mark.color}`, textUnderlineOffset: "0.2em" });
          if (p.animation === "word-pop" || p.animation === "pulse") st.transform = "scale(1.12)";
        } else if (p.animation === "spotlight" || p.id === "minimal-modern") st.opacity = p.animation === "spotlight" ? 0.4 : 0.72;
        if (p.animation === "stacked" && p.id === "poster-stack" && w.length === Math.max(...words.map((x) => x.length))) st.fontSize = "1.3em";
        return (
          <span key={i} style={st}>
            {w}
          </span>
        );
      })}
    </span>
  );
}

export interface WordTiming {
  word: string;
  start: number;
  end: number;
}

export interface CaptionSegment {
  id: string;
  text: string;
  startTime: number;
  endTime: number;
  wordTimings?: WordTiming[];
}

/**
 * A segment with new text. Word timings are kept when the words still line up one to one (a spelling fix); otherwise
 * they are spread over the segment by word length, so the spoken-word highlight follows the edited words.
 */
export function retimeSegment(seg: CaptionSegment, text: string): CaptionSegment {
  const words = text.split(/\s+/).filter(Boolean);
  const old = seg.wordTimings ?? [];
  if (old.length === words.length && old.length > 0) return { ...seg, text, wordTimings: old.map((t, i) => ({ ...t, word: words[i] })) };
  if (old.length === 0) return { ...seg, text };
  const total = words.reduce((n, w) => n + w.length + 1, 0) || 1;
  const span = seg.endTime - seg.startTime;
  let t = seg.startTime;
  const wordTimings = words.map((w) => {
    const start = t;
    t += (span * (w.length + 1)) / total;
    return { word: w, start: Math.round(start * 1000) / 1000, end: Math.round(t * 1000) / 1000 };
  });
  return { ...seg, text, wordTimings };
}

/** Languages offered for transcription (ISO 639-1); "auto" detects. */
export const CAPTION_LANGUAGES: [string, string][] = [
  ["auto", "Detect automatically"],
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["pl", "Polish"],
  ["tr", "Turkish"],
  ["ru", "Russian"],
  ["uk", "Ukrainian"],
  ["ar", "Arabic"],
  ["hi", "Hindi"],
  ["ml", "Malayalam"],
  ["ta", "Tamil"],
  ["ja", "Japanese"],
  ["ko", "Korean"],
  ["zh", "Chinese"],
  ["id", "Indonesian"],
  ["vi", "Vietnamese"],
];
