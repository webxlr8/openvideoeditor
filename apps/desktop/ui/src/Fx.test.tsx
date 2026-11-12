import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import type { Fields } from "./edit";
import catalogText from "./fx-catalog.json?raw";

async function editorWithClip() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  await screen.findByTestId("inspector");
}

const clips = () => screen.queryAllByTestId("clip");

/** Clicks the video clip, which brings the inspector back after a tool panel. */
async function inspectClip() {
  const clip = clips().find((c) => !c.textContent?.includes("Aurora"))!;
  fireEvent.pointerDown(clip, { button: 0, clientX: 300 });
  fireEvent.pointerUp(window);
  await screen.findByTestId("inspector");
}

async function items(): Promise<Fields[]> {
  const v = await commands.projectView();
  const ids = v.status === "ok" && v.data ? v.data.items.map((i) => i.id) : [];
  const out: Fields[] = [];
  for (const id of ids) {
    const d = await commands.itemDetail(id);
    if (d.status === "ok") out.push(JSON.parse(d.data) as Fields);
  }
  return out;
}

async function clipDetail(): Promise<Fields> {
  return (await items()).find((i) => i.type === "video")!;
}

describe("effects, transitions, layouts and colour", () => {
  it("the catalogue matches the contract counts", () => {
    const c = JSON.parse(catalogText) as { effects: unknown[]; layouts: unknown[]; luts: unknown[]; transitions: { id: string }[] };
    expect(c.effects.length).toBe(41);
    expect(c.layouts.length).toBe(15);
    expect(c.luts.length).toBe(9);
    for (const t of ["wipe-left", "push", "cross-zoom", "audio-fade", "radial-wipe", "ink-bleed", "film-burn"]) {
      expect(c.transitions.some((x) => x.id === t)).toBe(true);
    }
  });

  it("adds an effect on a clip and edits a parameter generated from its contract", async () => {
    await editorWithClip();
    fireEvent.click(screen.getByTestId("rail-effects"));
    fireEvent.click(await screen.findByTestId("effect-duotone-clip"));
    await waitFor(async () => expect(((await clipDetail()).effects as Fields[] | undefined)?.[0]?.type).toBe("duotone"));
    const entry = (await clipDetail()).effects as Fields[];
    expect(entry[0].balance).toBe(0.5);
    expect(entry[0].shadows).toEqual([0.09, 0.11, 0.36]);
    await inspectClip();
    fireEvent.click(screen.getByTestId("insp-tab-effects"));
    const balance = await screen.findByTestId<HTMLInputElement>("fx-duotone-balance");
    fireEvent.change(balance, { target: { value: "0.8" } });
    fireEvent.pointerUp(balance);
    await waitFor(async () => expect(((await clipDetail()).effects as Fields[])[0].balance).toBe(0.8));
    fireEvent.click(screen.getByTestId("fx-remove-duotone"));
    await waitFor(async () => expect((await clipDetail()).effects).toEqual([]));
  });

  it("adds a background generator as a shader layer with its own inspector", async () => {
    await editorWithClip();
    fireEvent.click(screen.getByTestId("rail-effects"));
    fireEvent.click(await screen.findByTestId("effect-aurora-layer"));
    await waitFor(() => expect(clips().length).toBe(2));
    const shader = (await items()).find((i) => i.type === "shader")!;
    expect((shader.shaderComposition as Fields).template).toBe("aurora");
    const layer = clips().find((c) => c.textContent?.includes("Aurora"))!;
    fireEvent.pointerDown(layer, { button: 0, clientX: 300 });
    fireEvent.pointerUp(window);
    expect(await screen.findByTestId("shader-editor")).toBeTruthy();
    expect(screen.getByTestId("shader-intensity")).toBeTruthy();
  });

  it("applies a directional transition with easing and duration", async () => {
    await editorWithClip();
    fireEvent.click(screen.getByTestId("rail-transitions"));
    fireEvent.click(await screen.findByTestId("transition-dir-right"));
    fireEvent.change(screen.getByTestId("transition-easing"), { target: { value: "ease-out" } });
    const card = await screen.findByTestId("transition-wipe");
    fireEvent.click(card.querySelector("button")!);
    await waitFor(async () => expect((await clipDetail()).transitionIn).toEqual({ type: "wipe", duration: 0.5, easing: "ease-out", direction: "right" }));
    // The inspector edits the same transition.
    await inspectClip();
    fireEvent.click(screen.getByTestId("insp-tab-general"));
    fireEvent.click(await screen.findByTestId("insp-transitionIn-dir-up"));
    await waitFor(async () => expect(((await clipDetail()).transitionIn as Fields).direction).toBe("up"));
  });

  it("places selected clips into a layout's slots", async () => {
    await editorWithClip();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(clips().length).toBe(2));
    const other = clips().find((c) => !c.classList.contains("selected"))!;
    fireEvent.pointerDown(other, { button: 0, shiftKey: true });
    await waitFor(() => expect(clips().filter((c) => c.classList.contains("selected")).length).toBe(2));
    fireEvent.click(screen.getByTestId("rail-layout"));
    fireEvent.click(await screen.findByTestId("layout-split-horizontal"));
    await waitFor(async () => {
      const slots = (await items()).map((i) => (i.layout as Fields | undefined)?.slot).sort();
      expect(slots).toEqual([0, 1]);
    });
    expect(((await items())[0].layout as Fields).preset).toBe("split-horizontal");
  });

  it("grades a clip: look LUT, wheel, curve and match to frame", async () => {
    await editorWithClip();
    fireEvent.click(screen.getByTestId("insp-tab-color"));
    fireEvent.click(await screen.findByTestId("lut-cinematic"));
    await waitFor(async () => expect(((await clipDetail()).colorGrade as Fields).lut).toEqual({ intensity: 1, stage: "look", preset: "cinematic" }));
    const y = await screen.findByTestId<HTMLInputElement>("wheel-gain-y");
    fireEvent.change(y, { target: { value: "0.2" } });
    fireEvent.pointerUp(y);
    await waitFor(async () => expect((((await clipDetail()).colorGrade as Fields).wheels as Fields).gain).toEqual({ r: 0, g: 0, b: 0, y: 0.2 }));
    fireEvent.click(screen.getByTestId("curve-tab-red"));
    expect(screen.getByTestId("curve-red")).toBeTruthy();
    fireEvent.click(screen.getByTestId("grade-match"));
    await waitFor(async () => expect(((((await clipDetail()).colorGrade as Fields).wheels as Fields).gain as Fields).r).toBe(0.08));
    expect(((await clipDetail()).colorGrade as Fields).lut).toBeTruthy();
    expect(await screen.findByTestId("scopes")).toBeTruthy();
  });
});
