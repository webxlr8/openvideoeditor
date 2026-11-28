import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import { ease, laneProperties, valueAt } from "./KeyframeLanes";

async function view() {
  const v = await commands.projectView();
  if (v.status !== "ok" || !v.data) throw new Error("no project");
  return v.data;
}

/** The editor with its clip carrying opacity keyframes 1 at 0 s and 0 at 2 s. */
async function editorWithKeyframes() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  const clip = (await view()).items[0];
  await act(async () => {
    await commands.keyframesSet(clip.id, "opacity", JSON.stringify([{ time: 0, value: 1 }, { time: 2, value: 0 }]), null);
  });
  const track = (await view()).tracks.find((t) => t.id === clip.trackId)!;
  return { clip, trackName: track.name };
}

const opacityKeys = async () => (await view()).items[0].keyframes?.find((t) => t.property === "opacity")?.keys ?? [];
const diamonds = () => screen.getAllByTestId("kf-diamond");

describe("keyframe lanes", () => {
  it("expanding a track shows its lanes and creates no revision", async () => {
    const { trackName } = await editorWithKeyframes();
    const before = (await view()).rev;
    expect(screen.queryByTestId("kf-lanes")).toBeNull();
    fireEvent.click(screen.getByTestId(`kf-toggle-${trackName}`));
    const lanes = await screen.findByTestId("kf-lanes");
    expect(within(lanes).getByTestId("kf-lane-opacity").textContent).toContain("Opacity");
    expect(diamonds()).toHaveLength(2);
    expect((await view()).rev).toBe(before);
    fireEvent.click(screen.getByTestId(`kf-toggle-${trackName}`));
    expect(screen.queryByTestId("kf-lanes")).toBeNull();
    expect((await view()).rev).toBe(before);
  });

  it("dragging a diamond moves the keyframe later in one revision", async () => {
    const { trackName } = await editorWithKeyframes();
    fireEvent.click(screen.getByTestId(`kf-toggle-${trackName}`));
    const d = diamonds().find((x) => x.dataset.time === "2")!;
    const pps = parseFloat(d.style.left) / 2;
    const before = (await view()).rev ?? 0;
    fireEvent.pointerDown(d, { button: 0, clientX: 100, clientY: 10 });
    fireEvent.pointerMove(window, { clientX: 100 + 0.5 * pps, clientY: 10 });
    fireEvent.pointerUp(window, { clientX: 100 + 0.5 * pps, clientY: 10 });
    await waitFor(async () => expect((await opacityKeys()).map((k) => k.time)).toEqual([0, 2.5]));
    expect((await view()).rev).toBe(before + 1);
  });

  it("Alt-click adds a keyframe at the current value and Delete removes one, one revision each", async () => {
    const { trackName } = await editorWithKeyframes();
    fireEvent.click(screen.getByTestId(`kf-toggle-${trackName}`));
    const pps = parseFloat(diamonds().find((x) => x.dataset.time === "2")!.style.left) / 2;
    const start = (await view()).items[0].start ?? 0;
    const before = (await view()).rev ?? 0;
    fireEvent.pointerDown(screen.getByTestId("kf-body-opacity"), { button: 0, altKey: true, clientX: (start + 1) * pps });
    await waitFor(async () => expect(await opacityKeys()).toHaveLength(3));
    const added = (await opacityKeys())[1];
    expect(added.time).toBeCloseTo(1, 2);
    expect(added.value).toBeCloseTo(0.5, 2);
    expect((await view()).rev).toBe(before + 1);
    await waitFor(() => expect(diamonds()).toHaveLength(3));
    fireEvent.keyDown(diamonds()[0], { key: "Delete" });
    await waitFor(async () => expect(await opacityKeys()).toHaveLength(2));
    expect((await view()).rev).toBe(before + 2);
    expect(screen.getAllByTestId("clip")).toHaveLength(1);
  });

  it("evaluates easings like the renderer", () => {
    expect(ease("hold", 0.99)).toBe(0);
    expect(ease("hold", 1)).toBe(1);
    expect(ease("ease-in", 0.5)).toBe(0.25);
    const keys = [
      { time: 0, value: 1, easing: "hold" },
      { time: 2, value: 0, easing: null },
    ];
    expect(valueAt(keys, "linear", 1.9)).toBe(1);
    expect(valueAt(keys, "linear", 2)).toBe(0);
    expect(laneProperties([])).toEqual([]);
  });
});
