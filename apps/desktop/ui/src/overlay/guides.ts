// Viewer guides (add-viewer-interaction-and-text-fills, design D4): grids, safe areas, a centre cross, format masks
// and platform UI-safe zones. They are view state, never document data: kept per project in this browser's storage
// and drawn over the viewer only, so an export never contains them. `ove look --overlay` draws the same guides into
// agent frames (crates/ove-analyze/src/guides.rs uses the same numbers).

/** Inset of the action-safe area from each edge, as a fraction of the frame (design D4). */
export const ACTION_SAFE = 0.035;
/** Inset of the title-safe area from each edge. */
export const TITLE_SAFE = 0.05;

export const MASKS = ["2.39:1", "1.85:1", "1:1", "9:16", "4:5"] as const;
export type Mask = (typeof MASKS)[number];

/** A platform UI-safe zone: the share of the frame (percent) its interface may cover on each side. */
export type Preset = { id: string; name: string; top: number; right: number; bottom: number; left: number };

/** Starting presets for vertical video: our own measurements, editable (platforms change their layouts). */
export const DEFAULT_PRESETS: Preset[] = [
  { id: "vertical-feed", name: "Vertical feed", top: 12, right: 14, bottom: 22, left: 4 },
  { id: "vertical-stories", name: "Vertical stories", top: 14, right: 4, bottom: 18, left: 4 },
];

export type Guides = {
  thirds: boolean;
  /** Divisions of the grid (2 to 5), 0 for none. */
  grid: number;
  actionSafe: boolean;
  titleSafe: boolean;
  centre: boolean;
  mask: Mask | null;
  /** Preset id of the platform zone shown, if any. */
  platform: string | null;
};

export const NO_GUIDES: Guides = {
  thirds: false,
  grid: 0,
  actionSafe: false,
  titleSafe: false,
  centre: false,
  mask: null,
  platform: null,
};

export function anyGuide(g: Guides): boolean {
  return g.thirds || g.grid > 0 || g.actionSafe || g.titleSafe || g.centre || g.mask !== null || g.platform !== null;
}

/** "2.39:1" → 2.39, "9:16" → 0.5625. */
export function ratio(mask: Mask): number {
  const [a, b] = mask.split(":").map(Number);
  return a / b;
}

/**
 * The part of a `w`×`h` frame a format mask keeps, centred: letterboxed when the mask is wider than the frame,
 * pillarboxed when narrower.
 */
export function maskRect(mask: Mask, w: number, h: number): { x: number; y: number; w: number; h: number } {
  const r = ratio(mask);
  if (r >= w / h) {
    const mh = w / r;
    return { x: 0, y: (h - mh) / 2, w, h: mh };
  }
  const mw = h * r;
  return { x: (w - mw) / 2, y: 0, w: mw, h };
}

/** Lines (positions along one axis, 0..1) of a grid with `n` divisions. */
export function gridLines(n: number): number[] {
  return Array.from({ length: Math.max(0, n - 1) }, (_, i) => (i + 1) / n);
}

// Per-project guides and the presets live in this browser's storage: a convenience, never project data.
const KEY = "ove.guides.";
const PRESETS_KEY = "ove.guidePresets";

export function loadGuides(projectId: string | null | undefined): Guides {
  if (!projectId) return NO_GUIDES;
  try {
    const raw = localStorage.getItem(KEY + projectId);
    return raw ? { ...NO_GUIDES, ...(JSON.parse(raw) as Partial<Guides>) } : NO_GUIDES;
  } catch {
    return NO_GUIDES;
  }
}

export function saveGuides(projectId: string | null | undefined, g: Guides) {
  if (!projectId) return;
  try {
    localStorage.setItem(KEY + projectId, JSON.stringify(g));
  } catch {
    // Storage may be unavailable (private mode); the guides then last for this session only.
  }
}

export function loadPresets(): Preset[] {
  try {
    const raw = localStorage.getItem(PRESETS_KEY);
    const list = raw ? (JSON.parse(raw) as Preset[]) : null;
    return Array.isArray(list) && list.length > 0 ? list : DEFAULT_PRESETS;
  } catch {
    return DEFAULT_PRESETS;
  }
}

export function savePresets(list: Preset[]) {
  try {
    localStorage.setItem(PRESETS_KEY, JSON.stringify(list));
  } catch {
    // As above.
  }
}

/** Keeps a preset's insets in 0..90 % and its opposite sides from overlapping. */
export function cleanPreset(p: Preset): Preset {
  const c = (v: number) => Math.min(90, Math.max(0, Number.isFinite(v) ? Math.round(v * 10) / 10 : 0));
  const out = { ...p, top: c(p.top), right: c(p.right), bottom: c(p.bottom), left: c(p.left) };
  if (out.top + out.bottom > 95) out.bottom = 95 - out.top;
  if (out.left + out.right > 95) out.right = 95 - out.left;
  return out;
}
