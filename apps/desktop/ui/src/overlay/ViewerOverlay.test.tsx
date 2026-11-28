// The viewer overlay against the browser mock (add-viewer-interaction-and-text-fills tasks 1.3 and 2.2): click to
// select by drawn pixels, one revision per handle drag, undo, Esc cancel, crop along a rotated layer's own edge, and
// no handles during playback. The mock's geometry is overlay/overlayMock.ts; the core's own hit test and geometry are
// tested in crates/ove-render/tests/geometry.rs.

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { ProjectView_Serialize } from "../bindings";
import { commands } from "../bridge";
import type { EditResult, Fields } from "../edit";
import { addItems, updateItem } from "../edit";
import { fire } from "../notify";
import { ViewerOverlay } from "./ViewerOverlay";

const W = 320;
const H = 180;

async function project(items: Fields[]): Promise<ProjectView_Serialize> {
  await commands.createProject("Overlay", null);
  const r = await addItems(items, "setup");
  if (r.status !== "ok") throw new Error(r.error.message);
  return r.data;
}

const CLIP: Fields = { id: "clip", trackId: "t_main", type: "video", startTime: 0, endTime: 5, mediaItemId: "m" };
const TITLE: Fields = { id: "title", trackId: "t_text", type: "text", startTime: 0, endTime: 5, textContent: "I I" };

function Harness({ start, playing = false, selected = [] }: { start: ProjectView_Serialize; playing?: boolean; selected?: string[] }) {
  const [p, setP] = useState(start);
  const [selection, setSelection] = useState<string[]>(selected);
  const onEdit = (e: EditResult) => {
    fire(
      e.then((r) => {
        if (r.status === "ok") setP(r.data);
      }),
    );
  };
  return (
    <div>
      <div data-testid="selection">{selection.join(",")}</div>
      <ViewerOverlay
        project={p}
        time={1}
        playing={playing}
        selection={selection}
        width={W}
        height={H}
        onSelect={setSelection}
        onEdit={onEdit}
        onError={(e) => {
          throw new Error(e.message);
        }}
      />
    </div>
  );
}

function overlay() {
  const el = screen.getByTestId("viewer-overlay");
  el.getBoundingClientRect = () => ({ left: 0, top: 0, width: W, height: H, right: W, bottom: H, x: 0, y: 0, toJSON: () => ({}) });
  return el;
}

function click(x: number, y: number) {
  const el = overlay();
  fireEvent.pointerDown(el, { button: 0, clientX: x, clientY: y });
  fireEvent.pointerUp(el, { button: 0, clientX: x, clientY: y });
}

/** A drag on `target` (a handle, or the overlay for a move) from (x0, y0) to (x1, y1) in steps. */
function drag(target: Element, x0: number, y0: number, x1: number, y1: number, opts: { shift?: boolean } = {}) {
  const el = overlay();
  fireEvent.pointerDown(target, { button: 0, clientX: x0, clientY: y0, shiftKey: opts.shift });
  for (let i = 1; i <= 4; i++) {
    const t = i / 4;
    fireEvent.pointerMove(el, { clientX: x0 + (x1 - x0) * t, clientY: y0 + (y1 - y0) * t, shiftKey: opts.shift });
  }
  fireEvent.pointerUp(el, { button: 0, clientX: x1, clientY: y1, shiftKey: opts.shift });
}

async function detail(id: string): Promise<Fields> {
  const r = await commands.itemDetail(id);
  if (r.status !== "ok") throw new Error("gone");
  return JSON.parse(r.data) as Fields;
}

async function rev(): Promise<number> {
  const v = await commands.projectView();
  return v.status === "ok" && v.data ? (v.data.rev ?? 0) : 0;
}

const calls = () => window.__oveMock!.calls;

describe("viewer overlay", () => {
  it("selects by drawn pixels: a transparent part of a title selects the clip below", async () => {
    // The clip is half size, so the frame's corners show nothing.
    const p = await project([{ ...CLIP, transform: { scale: 0.5 } }, TITLE]);
    render(<Harness start={p} />);
    // The middle of "I I" is a gap in the title: the clip is chosen.
    click(160, 90);
    await waitFor(() => expect(screen.getByTestId("selection").textContent).toBe("clip"));
    expect(calls()).toContain("viewerPick:1.00:0.50:0.50");
    // On a letter: the title.
    click(120, 90);
    await waitFor(() => expect(screen.getByTestId("selection").textContent).toBe("title"));
    // Where nothing is drawn: the selection clears.
    click(5, 5);
    await waitFor(() => expect(screen.getByTestId("selection").textContent).toBe(""));
  });

  it("moves a layer with one revision, snaps to the centre, and undo restores it", async () => {
    const p = await project([{ ...CLIP, transform: { scale: 0.5 } }]);
    render(<Harness start={p} selected={["clip"]} />);
    await screen.findByTestId("overlay-handles");
    const before = await rev();
    // 60 px right on a 320 px viewer of a 1920 px frame: 360 reference pixels.
    drag(overlay(), 160, 90, 220, 90);
    await waitFor(async () => expect(((await detail("clip")).transform as Fields).x).toBe(360));
    expect(await rev()).toBe(before + 1);
    expect(calls().some((c) => c.startsWith("viewerPreview:clip:"))).toBe(true);
    // A drag ending 3 px from the centre snaps back onto it (and is still one revision).
    drag(overlay(), 220, 90, 163, 92);
    await waitFor(async () => expect(((await detail("clip")).transform as Fields).x).toBe(0));
    expect(((await detail("clip")).transform as Fields).y).toBe(0);
    expect(await rev()).toBe(before + 2);
    await act(async () => {
      await commands.undo();
    });
    expect(((await detail("clip")).transform as Fields).x).toBe(360);
  });

  it("Esc during a drag puts the layer back and makes no revision", async () => {
    const p = await project([{ ...CLIP, transform: { scale: 0.5 } }]);
    render(<Harness start={p} selected={["clip"]} />);
    await screen.findByTestId("overlay-handles");
    const before = await rev();
    const el = overlay();
    fireEvent.pointerDown(el, { button: 0, clientX: 160, clientY: 90 });
    fireEvent.pointerMove(el, { clientX: 200, clientY: 120 });
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(el, { button: 0, clientX: 200, clientY: 120 });
    await waitFor(() => expect(calls()[calls().length - 1]).toBe("viewerPreview:none"));
    expect(await rev()).toBe(before);
    expect((await detail("clip")).transform).toEqual({ scale: 0.5 });
  });

  it("scales from a corner and rotates in 15° steps with Shift", async () => {
    const p = await project([{ ...CLIP, transform: { scale: 0.5 } }]);
    render(<Harness start={p} selected={["clip"]} />);
    await screen.findByTestId("overlay-handles");
    // The bottom-right corner (240, 135) dragged out to double its distance from the centre (160, 90).
    drag(screen.getByTestId("handle-scale-2"), 240, 135, 320, 180);
    await waitFor(async () => expect(((await detail("clip")).transform as Fields).scale).toBe(1));
    const handle = await screen.findByTestId("handle-rotate");
    const [cx, cy] = [Number(handle.getAttribute("cx")), Number(handle.getAttribute("cy"))];
    // From straight above the centre to about 40° clockwise: 45° with Shift.
    const a = (40 * Math.PI) / 180;
    const r = 90 - cy;
    drag(handle, cx, cy, 160 + r * Math.sin(a), 90 - r * Math.cos(a), { shift: true });
    await waitFor(async () => expect(((await detail("clip")).transform as Fields).rotation).toBe(45));
  });

  it("crops a rotated layer along its own right edge", async () => {
    const p = await project([{ ...CLIP, transform: { scale: 0.5, rotation: 30 } }]);
    render(<Harness start={p} selected={["clip"]} />);
    const handle = await screen.findByTestId("handle-crop-right");
    const [x, y] = [Number(handle.getAttribute("cx")), Number(handle.getAttribute("cy"))];
    // Along the layer's own x axis (30° below horizontal), inwards by 20 % of its 160 px width.
    const d = 0.2 * 160;
    const [ux, uy] = [Math.cos(Math.PI / 6), Math.sin(Math.PI / 6)];
    drag(handle, x, y, x - ux * d, y - uy * d);
    await waitFor(async () => expect(((await detail("clip")).crop as Fields).right).toBeCloseTo(20, 0));
    const crop = (await detail("clip")).crop as Fields;
    expect([crop.top, crop.bottom, crop.left]).toEqual([0, 0, 0]);
    // A drag across the edge (along the layer's own y axis) does not change the right crop.
    const h2 = screen.getByTestId("handle-crop-right");
    const [x2, y2] = [Number(h2.getAttribute("cx")), Number(h2.getAttribute("cy"))];
    drag(h2, x2, y2, x2 - uy * 30, y2 + ux * 30);
    await waitFor(async () => expect(((await detail("clip")).crop as Fields).right).toBeCloseTo(20, 0));
  });

  it("titles get no crop handles; handles hide during playback", async () => {
    const p = await project([TITLE]);
    const { rerender } = render(<Harness start={p} selected={["title"]} />);
    await screen.findByTestId("overlay-handles");
    expect(screen.queryByTestId("handle-crop-right")).toBeNull();
    expect(screen.getByTestId("handle-rotate")).toBeTruthy();
    rerender(<Harness start={p} selected={["title"]} playing />);
    await waitFor(() => expect(screen.queryByTestId("overlay-handles")).toBeNull());
    expect(screen.getByTestId("viewer-overlay").getAttribute("data-playing")).toBe("true");
  });

  it("a plain update still works through the edit helper", async () => {
    // Guards the test project: the mock applies transform edits the way the overlay sends them.
    await project([CLIP]);
    const r = await updateItem("clip", { transform: { x: 10, y: 0, scale: 1, rotation: 0 } }, [], "move");
    expect(r.status).toBe("ok");
    expect(((await detail("clip")).transform as Fields).x).toBe(10);
  });
});
