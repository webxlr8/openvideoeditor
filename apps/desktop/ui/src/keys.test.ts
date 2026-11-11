import { describe, expect, it } from "vitest";
import { keyToAction, type KeyLike } from "./keys";

const key = (k: string, extra: Partial<KeyLike> = {}): KeyLike => ({
  key: k,
  shiftKey: false,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  target: document.body,
  ...extra,
});

describe("keyboard map", () => {
  it("maps J/K/L, space, arrows, Home/End", () => {
    expect(keyToAction(key(" "))).toEqual({ type: "transport", command: { kind: "togglePlay" } });
    expect(keyToAction(key("j"))).toEqual({ type: "transport", command: { kind: "shuttleReverse" } });
    expect(keyToAction(key("k"))).toEqual({ type: "transport", command: { kind: "pause" } });
    expect(keyToAction(key("L"))).toEqual({ type: "transport", command: { kind: "shuttleForward" } });
    expect(keyToAction(key("ArrowRight"))).toEqual({ type: "transport", command: { kind: "step", frames: 1 } });
    expect(keyToAction(key("ArrowLeft", { shiftKey: true }))).toEqual({
      type: "transport",
      command: { kind: "step", frames: -10 },
    });
    expect(keyToAction(key("Home"))).toEqual({ type: "transport", command: { kind: "goToStart" } });
    expect(keyToAction(key("End"))).toEqual({ type: "transport", command: { kind: "goToEnd" } });
  });

  it("ignores keys typed into text fields and with modifiers", () => {
    const input = document.createElement("input");
    expect(keyToAction(key(" ", { target: input }))).toBeNull();
    expect(keyToAction(key("l", { metaKey: true }))).toBeNull();
    expect(keyToAction(key("x"))).toBeNull();
  });

  it("toggles diagnostics with Cmd/Ctrl+Shift+D even in text fields", () => {
    const input = document.createElement("textarea");
    expect(keyToAction(key("D", { ctrlKey: true, shiftKey: true, target: input }))).toEqual({
      type: "toggleDiagnostics",
    });
  });
});
