// Browser-mock data for the motion-graphics and font commands (UI development, Playwright, unit tests). The real
// app gets these from ove-graphics; the shapes match.

import type { FontFamily } from "./bindings";

const bundled = (family: string, category: string, minWeight: number, maxWeight: number, italic: boolean): FontFamily => ({
  family,
  source: "bundled",
  category,
  minWeight,
  maxWeight,
  italic,
});

export const MOCK_FONTS: FontFamily[] = [
  bundled("Anton", "display", 400, 400, false),
  bundled("Archivo Black", "display", 400, 400, false),
  bundled("Bebas Neue", "display", 400, 400, false),
  bundled("Caveat", "handwriting", 400, 700, false),
  bundled("DM Sans", "sans-serif", 100, 1000, false),
  bundled("Dancing Script", "handwriting", 400, 700, false),
  bundled("Instrument Serif", "serif", 400, 400, true),
  bundled("Inter", "sans-serif", 100, 900, true),
  bundled("JetBrains Mono", "mono", 100, 800, false),
  bundled("Lora", "serif", 400, 700, true),
  bundled("Montserrat", "sans-serif", 100, 900, true),
  bundled("Oswald", "display", 200, 700, false),
  bundled("Playfair Display", "serif", 400, 900, true),
  bundled("Poppins", "sans-serif", 400, 900, true),
  bundled("Roboto", "sans-serif", 100, 900, true),
  bundled("Space Grotesk", "sans-serif", 300, 700, false),
  { family: "DejaVu Sans", source: "system", category: "system", minWeight: 400, maxWeight: 700, italic: true },
];

const svgUrl = (w: number, h: number, body: string) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}">${body}</svg>`)}`;

const esc = (s: string) => s.replace(/[<>&"]/g, (c) => `&#${c.charCodeAt(0)};`);

export function mockFontPreview(family: string, text: string, size: number): string {
  const w = Math.max(10, Math.round(text.length * size * 0.62));
  const h = Math.round(size * 1.3);
  return svgUrl(w, h, `<text x="0" y="${size}" font-family="${esc(family)}, sans-serif" font-size="${size}" fill="#fff">${esc(text)}</text>`);
}

const MOCK_SOURCE = `export default function Mock({ time, props }) {
  return <Fill style={{ alignItems: "center", justifyContent: "center" }}><div style={{ opacity: clamp(time) }}>{props.title}</div></Fill>;
}
`;

export const MOCK_TEMPLATES = [
  {
    id: "lower-third",
    name: "Lower third",
    description: "Name and role beside a growing accent bar",
    duration: 5,
    fullFrame: false,
    props: [
      { name: "name", label: "Name", kind: "text", default: "Alex Morgan" },
      { name: "role", label: "Role", kind: "text", default: "Product designer" },
      { name: "accent", label: "Accent", kind: "color", default: "#ffb020" },
      { name: "side", label: "Side", kind: "select", default: "left", options: ["left", "right"] },
    ],
    source: MOCK_SOURCE,
  },
  {
    id: "title-card",
    name: "Title card",
    description: "Gradient background with a letter-by-letter title",
    duration: 4,
    fullFrame: true,
    props: [
      { name: "title", label: "Title", kind: "text", default: "Chapter One" },
      { name: "from", label: "Gradient start", kind: "color", default: "#1d2b64" },
    ],
    source: MOCK_SOURCE,
  },
  {
    id: "counter",
    name: "Counter",
    description: "A number counting up to a target",
    duration: 4,
    fullFrame: false,
    props: [
      { name: "to", label: "To", kind: "number", default: 12500, min: -1e9, max: 1e9, step: 1 },
      { name: "suffix", label: "Suffix", kind: "text", default: "+" },
    ],
    source: MOCK_SOURCE,
  },
  {
    id: "logo-sting",
    name: "Logo sting",
    description: "Logo springs in with a ring burst and tagline",
    duration: 3,
    fullFrame: true,
    props: [
      { name: "logo", label: "Logo image", kind: "image", default: null },
      { name: "tagline", label: "Tagline", kind: "text", default: "MAKE SOMETHING GOOD" },
    ],
    source: MOCK_SOURCE,
  },
];

export function mockThumbnail(id: string): string | null {
  const t = MOCK_TEMPLATES.find((x) => x.id === id);
  if (!t) return null;
  return svgUrl(288, 162, `<rect width="288" height="162" fill="#1d2b64"/><text x="144" y="88" text-anchor="middle" font-family="sans-serif" font-size="20" fill="#fff">${esc(t.name)}</text>`);
}

/** A rough stand-in for ove-graphics' check: flags imports, missing default export and endless loops. */
export function mockCheck(compositionJson: string): string {
  const c = JSON.parse(compositionJson) as { source?: string; template?: string };
  const diagnostics: { severity: string; code: string; message: string; line: number | null; column: number | null }[] = [];
  if (c.template && !MOCK_TEMPLATES.some((t) => t.id === c.template)) diagnostics.push({ severity: "error", code: "template", message: `unknown template \`${c.template}\``, line: null, column: null });
  const src = c.source ?? "";
  src.split("\n").forEach((l, i) => {
    const imp = l.indexOf("import ");
    if (imp >= 0) diagnostics.push({ severity: "error", code: "import", message: "imports are not supported: write everything in this one file", line: i + 1, column: imp + 1 });
    const svg = l.indexOf("<svg");
    if (svg >= 0) diagnostics.push({ severity: "warning", code: "tag", message: "<svg> is not supported", line: i + 1, column: svg + 1 });
  });
  if (c.source !== undefined && !src.includes("export default")) diagnostics.push({ severity: "error", code: "export", message: "missing `export default function`", line: null, column: null });
  if (src.includes("for (;;)")) diagnostics.push({ severity: "error", code: "budget", message: "the composition ran longer than its 250 ms budget (endless loop?)", line: null, column: null });
  return JSON.stringify({ ok: !diagnostics.some((d) => d.severity === "error"), diagnostics, logs: [], frameMs: 6, width: 1920, height: 1080 });
}
