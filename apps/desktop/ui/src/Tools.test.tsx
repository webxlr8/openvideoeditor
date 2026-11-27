import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import { keyToAction, resetBindings, setBinding, type KeyLike } from "./keys";

async function editor() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
}

const clips = () => screen.queryAllByTestId("clip");
const calls = () => window.__oveMock!.calls;
const press = (key: string, extra: Record<string, unknown> = {}) => fireEvent.keyDown(window, { key, ...extra });
const tool = () => screen.getByTestId("timeline").getAttribute("data-tool");

/** Puts the playhead inside the only clip (clicking it seeks there and selects it). */
async function selectClipAt(clientX = 400) {
  fireEvent.pointerDown(clips()[0], { button: 0, clientX });
  fireEvent.pointerUp(window);
  await waitFor(() => expect(screen.getByTestId("timecode").textContent).not.toBe("00:00:00:00"));
}

const key = (k: string, extra: Partial<KeyLike> = {}): KeyLike => ({
  key: k,
  shiftKey: false,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  target: document.body,
  ...extra,
});

afterEach(() => resetBindings());

describe("edit keys", () => {
  it("map the tools, marks, ripple trims, ripple delete, insert and overwrite", () => {
    expect(keyToAction(key("v"))).toEqual({ type: "tool", tool: "select" });
    expect(keyToAction(key("B"))).toEqual({ type: "tool", tool: "blade" });
    expect(keyToAction(key("t"))).toEqual({ type: "tool", tool: "trim" });
    expect(keyToAction(key("i"))).toEqual({ type: "markIn" });
    expect(keyToAction(key("o"))).toEqual({ type: "markOut" });
    expect(keyToAction(key("x"))).toEqual({ type: "markClip" });
    expect(keyToAction(key("q"))).toEqual({ type: "rippleTrimStart" });
    expect(keyToAction(key("w"))).toEqual({ type: "rippleTrimEnd" });
    expect(keyToAction(key(","))).toEqual({ type: "insert" });
    expect(keyToAction(key("."))).toEqual({ type: "overwrite" });
    expect(keyToAction(key("Delete", { shiftKey: true }))).toEqual({ type: "rippleDelete" });
    expect(keyToAction(key("Delete"))).toEqual({ type: "delete" });
  });

  it("are rebindable", () => {
    setBinding("toolBlade", "n");
    expect(keyToAction(key("n"))).toEqual({ type: "tool", tool: "blade" });
    expect(keyToAction(key("b"))).toBeNull();
    setBinding("rippleTrimEnd", "e");
    expect(keyToAction(key("e"))).toEqual({ type: "rippleTrimEnd" });
  });
});

describe("tool modes", () => {
  it("V, B and T switch the tool, and the toolbar shows it", async () => {
    await editor();
    expect(tool()).toBe("select");
    press("b");
    await waitFor(() => expect(tool()).toBe("blade"));
    expect(screen.getByTestId("tool-blade").getAttribute("aria-pressed")).toBe("true");
    press("t");
    await waitFor(() => expect(tool()).toBe("trim"));
    press("v");
    await waitFor(() => expect(tool()).toBe("select"));
    fireEvent.click(screen.getByTestId("tool-trim"));
    await waitFor(() => expect(tool()).toBe("trim"));
  });

  it("the blade cuts a clip where it is clicked", async () => {
    await editor();
    press("b");
    await waitFor(() => expect(tool()).toBe("blade"));
    fireEvent.pointerDown(clips()[0], { button: 0, clientX: 400 });
    await waitFor(() => expect(clips().length).toBe(2));
  });

  it("in the trim tool a drag slips, an Alt-drag slides and an edit point rolls", async () => {
    await editor();
    press("t");
    await waitFor(() => expect(tool()).toBe("trim"));
    fireEvent.pointerDown(clips()[0], { button: 0, clientX: 300 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 340 }));
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 340 }));
    });
    await waitFor(() => expect(calls()).toContain("editPlan:slip"));
    fireEvent.pointerDown(clips()[0], { button: 0, clientX: 300 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 340, altKey: true }));
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 340, altKey: true }));
    });
    await waitFor(() => expect(calls()).toContain("editPlan:slide"));
    // Two touching clips: cut one with the blade, then drag the edit point in the trim tool.
    press("b");
    fireEvent.pointerDown(clips()[0], { button: 0, clientX: 400 });
    await waitFor(() => expect(clips().length).toBe(2));
    press("t");
    await waitFor(() => expect(tool()).toBe("trim"));
    const left = clips().sort((a, b) => a.offsetLeft - b.offsetLeft)[0];
    fireEvent.pointerDown(left, { button: 0, clientX: 300 });
    fireEvent.pointerUp(window);
    const handle = await screen.findByTestId("trim-end");
    fireEvent.pointerDown(handle, { button: 0, clientX: 400 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 430 }));
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 430 }));
    });
    await waitFor(() => expect(calls()).toContain("editPlan:roll"));
  });
});

describe("range and ripple keys", () => {
  it("I and O mark a range, X marks the clip, Shift+Delete extracts the range", async () => {
    await editor();
    await selectClipAt(400);
    press("i");
    await waitFor(() => expect(screen.getByTestId("range-in")).toBeTruthy());
    press("ArrowRight", { shiftKey: true });
    press("ArrowRight", { shiftKey: true });
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).not.toBe(""));
    press("o");
    const range = await screen.findByTestId("range");
    expect(Number(range.getAttribute("data-out"))).toBeGreaterThan(Number(range.getAttribute("data-in")));
    press("Delete", { shiftKey: true });
    await waitFor(() => expect(calls()).toContain("editPlan:extract"));
    await waitFor(() => expect(screen.queryByTestId("range")).toBeNull());
    press("x");
    const marked = await screen.findByTestId("range");
    expect(Number(marked.getAttribute("data-in"))).toBe(0);
  });

  it("Q and W ripple trim to the playhead; Shift+Delete ripple deletes the selection", async () => {
    await editor();
    await selectClipAt(400);
    press("q");
    await waitFor(() => expect(calls().filter((c) => c === "editPlan:rippleTrim").length).toBe(1));
    press("w");
    await waitFor(() => expect(calls().filter((c) => c === "editPlan:rippleTrim").length).toBe(2));
    press("Delete", { shiftKey: true });
    await waitFor(() => expect(calls()).toContain("editPlan:rippleDelete"));
  });

  it(", and . insert and overwrite the source clip at the playhead", async () => {
    await editor();
    // With the timeline in the viewer there is no source: an error, nothing planned.
    press(",");
    await waitFor(() => expect(document.body.textContent).toContain("Show a clip from the project in the viewer first"));
    expect(calls()).not.toContain("editPlan:insert");
    // Show the clip's source, then insert and overwrite it.
    await selectClipAt(400);
    fireEvent.click(screen.getByTestId("preview-source"));
    await waitFor(() => expect(calls().some((c) => c.startsWith("openMedia:"))).toBe(true));
    await act(async () => {
      await commands.projectView();
    });
    press(",");
    await waitFor(() => expect(calls()).toContain("editPlan:insert"));
    press(".");
    await waitFor(() => expect(calls()).toContain("editPlan:overwrite"));
  });
});
