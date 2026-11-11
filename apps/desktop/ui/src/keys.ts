import type { TransportCommand } from "./bindings";

export type KeyAction =
  | { type: "transport"; command: TransportCommand }
  | { type: "toggleDiagnostics" }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "delete" };

export interface KeyLike {
  key: string;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  target: EventTarget | null;
}

function isTextEntry(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== "string") return false;
  const el = target as HTMLElement;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable;
}

/** Maps a key press to an editor action (J/K/L, space, arrows, Home/End, undo/redo, delete). */
export function keyToAction(e: KeyLike): KeyAction | null {
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.shiftKey && e.key.toLowerCase() === "d") return { type: "toggleDiagnostics" };
  if (isTextEntry(e.target)) return null;
  if (mod && !e.altKey && e.key.toLowerCase() === "z") return { type: e.shiftKey ? "redo" : "undo" };
  if (mod && !e.altKey && e.key.toLowerCase() === "y") return { type: "redo" };
  if (mod || e.altKey) return null;
  const t = (command: TransportCommand): KeyAction => ({ type: "transport", command });
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
    default:
      return null;
  }
}
