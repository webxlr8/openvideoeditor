import type { TransportCommand } from "./bindings";

/** The modifier key's name on this computer (⌘ on a Mac, Ctrl elsewhere). */
export const MOD =
  typeof navigator !== "undefined" && /mac/i.test(navigator.platform)
    ? "⌘"
    : "Ctrl";

export type KeyAction =
  | { type: "transport"; command: TransportCommand }
  | { type: "toggleDiagnostics" }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "delete" }
  | { type: "split" }
  | { type: "duplicate" }
  | { type: "copy" }
  | { type: "paste" }
  | { type: "selectAll" }
  | { type: "zoomIn" }
  | { type: "zoomOut" }
  | { type: "zoomFit" }
  | { type: "help" }
  | { type: "escape" };

/** Editing actions the editor handles itself (the app handles transport, undo/redo and diagnostics). */
export const EDIT_ACTIONS = new Set([
  "delete",
  "split",
  "duplicate",
  "copy",
  "paste",
  "selectAll",
  "zoomIn",
  "zoomOut",
  "zoomFit",
  "help",
  "escape",
]);

export interface KeyLike {
  key: string;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  target: EventTarget | null;
}

function isTextEntry(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== "string")
    return false;
  const el = target as HTMLElement;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    el.tagName === "SELECT" ||
    el.isContentEditable
  );
}

/** Single-key actions people can rebind (Settings → Shortcuts), with their default keys. */
export const REBINDABLE: { id: string; label: string; key: string }[] = [
  { id: "togglePlay", label: "Play / pause", key: " " },
  { id: "pause", label: "Pause", key: "k" },
  { id: "shuttleReverse", label: "Play backwards (repeat for 2×)", key: "j" },
  { id: "shuttleForward", label: "Play forwards (repeat for 2×)", key: "l" },
  { id: "split", label: "Split at the playhead", key: "s" },
  { id: "duplicate", label: "Duplicate selection", key: "d" },
  { id: "zoomIn", label: "Zoom in", key: "=" },
  { id: "zoomOut", label: "Zoom out", key: "-" },
  { id: "help", label: "Show shortcuts", key: "?" },
];

/** Keys that cannot be bound (they have fixed meanings). */
export const RESERVED = new Set([
  "Escape",
  "Delete",
  "Backspace",
  "ArrowLeft",
  "ArrowRight",
  "Home",
  "End",
  "Tab",
  "Enter",
]);

const STORE = "ove.shortcuts";
let custom: Record<string, string> = (() => {
  try {
    return JSON.parse(localStorage.getItem(STORE) ?? "{}") as Record<
      string,
      string
    >;
  } catch {
    return {};
  }
})();

/** The key bound to an action now (custom or default). */
export function bindingOf(id: string): string {
  return custom[id] ?? REBINDABLE.find((r) => r.id === id)?.key ?? "";
}

export function customBindings(): Record<string, string> {
  return { ...custom };
}

/** Binds `key` to an action (null resets it). A key already used by another action moves to this one. */
export function setBinding(id: string, key: string | null): void {
  const next = { ...custom };
  if (key === null) delete next[id];
  else {
    const k = key.length === 1 ? key.toLowerCase() : key;
    for (const r of REBINDABLE)
      if (r.id !== id && bindingOf(r.id) === k) next[r.id] = "";
    next[id] = k;
  }
  custom = next;
  try {
    localStorage.setItem(STORE, JSON.stringify(custom));
  } catch {
    // Not persisted (private window); the binding still applies until reload.
  }
}

export function resetBindings(): void {
  custom = {};
  try {
    localStorage.removeItem(STORE);
  } catch {
    // ignore
  }
}

/** Shown name of a key. */
export function keyLabel(k: string): string {
  if (k === " ") return "Space";
  if (k === "") return "—";
  return k.length === 1 ? k.toUpperCase() : k;
}

function rebindableAction(id: string): KeyAction | null {
  const t = (command: TransportCommand): KeyAction => ({
    type: "transport",
    command,
  });
  switch (id) {
    case "togglePlay":
      return t({ kind: "togglePlay" });
    case "pause":
      return t({ kind: "pause" });
    case "shuttleReverse":
      return t({ kind: "shuttleReverse" });
    case "shuttleForward":
      return t({ kind: "shuttleForward" });
    case "split":
    case "duplicate":
    case "zoomIn":
    case "zoomOut":
    case "help":
      return { type: id };
    default:
      return null;
  }
}

/** Maps a key press to an editor action (J/K/L, space, arrows, Home/End, undo/redo, delete). */
export function keyToAction(e: KeyLike): KeyAction | null {
  const plain = !(e.metaKey || e.ctrlKey || e.altKey) && !isTextEntry(e.target);
  if (plain && Object.keys(custom).length > 0) {
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const bound = REBINDABLE.find(
      (r) => custom[r.id] !== undefined && custom[r.id] === k,
    );
    if (bound) return rebindableAction(bound.id);
    // A default key whose action was moved elsewhere does nothing now.
    const moved = REBINDABLE.find(
      (r) =>
        custom[r.id] !== undefined &&
        (r.key === k ||
          (r.key === "=" && k === "+") ||
          (r.key === "-" && k === "_")),
    );
    if (moved && !RESERVED.has(e.key)) return null;
  }
  return defaultKeyToAction(e);
}

function defaultKeyToAction(e: KeyLike): KeyAction | null {
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.shiftKey && e.key.toLowerCase() === "d")
    return { type: "toggleDiagnostics" };
  if (isTextEntry(e.target)) return null;
  if (mod && !e.altKey && e.key.toLowerCase() === "z")
    return { type: e.shiftKey ? "redo" : "undo" };
  if (mod && !e.altKey && e.key.toLowerCase() === "y") return { type: "redo" };
  if (mod && !e.altKey && e.key.toLowerCase() === "c") return { type: "copy" };
  if (mod && !e.altKey && e.key.toLowerCase() === "v") return { type: "paste" };
  if (mod && !e.altKey && e.key.toLowerCase() === "a")
    return { type: "selectAll" };
  if (mod && !e.altKey && e.key === "0") return { type: "zoomFit" };
  if (mod || e.altKey) return null;
  const t = (command: TransportCommand): KeyAction => ({
    type: "transport",
    command,
  });
  switch (e.key) {
    case " ":
      return t({ kind: "togglePlay" });
    case "k":
    case "K":
      return t({ kind: "pause" });
    case "j":
    case "J":
      return t({ kind: "shuttleReverse" });
    case "l":
    case "L":
      return t({ kind: "shuttleForward" });
    case "ArrowLeft":
      return t({ kind: "step", frames: e.shiftKey ? -10 : -1 });
    case "ArrowRight":
      return t({ kind: "step", frames: e.shiftKey ? 10 : 1 });
    case "Home":
      return t({ kind: "goToStart" });
    case "End":
      return t({ kind: "goToEnd" });
    case "Delete":
    case "Backspace":
      return { type: "delete" };
    case "s":
    case "S":
      return { type: "split" };
    case "d":
    case "D":
      return { type: "duplicate" };
    case "=":
    case "+":
      return { type: "zoomIn" };
    case "-":
    case "_":
      return { type: "zoomOut" };
    case "?":
      return { type: "help" };
    case "Escape":
      return { type: "escape" };
    default:
      return null;
  }
}
