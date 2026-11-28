// The viewer overlay (add-viewer-interaction-and-text-fills, design D1–D3): SVG over the viewer rectangle, the same on
// every viewer backend. A click selects the topmost layer drawn under the pointer (the core judges by the layer's own
// pixels). The selected layer gets handles to move, scale, rotate and crop it while playback is paused; a drag
// previews live through the playback engine (no revision) and commits one revision on release. Esc cancels a drag.

import { useCallback, useEffect, useRef, useState } from "react";
import type { AppError, ProjectView_Serialize, ViewerGeometry, ViewerLayer } from "../bindings";
import { commands } from "../bridge";
import { type EditResult, updateItem } from "../edit";
import { fire, logError } from "../notify";
import {
  type Crop,
  type Pt,
  type Quad,
  type Side,
  type Transform,
  NO_CROP,
  NO_TRANSFORM,
  centre,
  contains,
  cropHandles,
  cropQuad,
  cropped,
  moved,
  referenceSize,
  rotateHandle,
  rotated,
  scaled,
  snapMove,
  sub,
  toBox,
  translate,
  turnScale,
} from "./geometry";

/** Pointer travel (screen pixels) under which a press is a click, not a drag. */
const CLICK_PX = 3;

type Handle = { kind: "move" } | { kind: "scale"; corner: number } | { kind: "rotate" } | { kind: "crop"; side: Side };

type Drag = {
  handle: Handle;
  itemId: string;
  start: Pt;
  at: Pt;
  shift: boolean;
  dragging: boolean;
  /** The layer's box when the drag began (screen pixels). */
  quad: Quad;
  cropQuad: Quad;
  transform: Transform;
  crop: Crop;
};

/** What a drag changes so far: the fields to set and the live outline. */
type Change = {
  set: Record<string, unknown>;
  summary: string;
  quad: Quad;
  cropQuad: Quad;
  lines: { x: number[]; y: number[] };
};

const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);

function readFields(detail: Record<string, unknown>): { transform: Transform; crop: Crop } {
  const t = (detail.transform as Record<string, unknown> | undefined) ?? {};
  const c = (detail.crop as Record<string, unknown> | undefined) ?? {};
  return {
    transform: {
      ...t,
      x: num(t.x, NO_TRANSFORM.x),
      y: num(t.y, NO_TRANSFORM.y),
      scale: num(t.scale, 1),
      rotation: num(t.rotation, 0),
    },
    crop: { top: num(c.top, 0), right: num(c.right, 0), bottom: num(c.bottom, 0), left: num(c.left, 0) },
  };
}

/** Which handles a layer of this item type gets: the renderer draws text without a crop, captions upright. */
export function handlesFor(itemType: string): { crop: boolean; rotate: boolean } {
  return {
    crop: itemType === "video" || itemType === "image" || itemType === "code",
    rotate: itemType !== "caption",
  };
}

/** The change a drag gives (pure: tested without a viewer). */
export function dragChange(d: Drag, w: number, h: number, refW: number): Change | null {
  const delta = sub(d.at, d.start);
  const c = centre(d.quad);
  switch (d.handle.kind) {
    case "move": {
      const snap = snapMove(d.cropQuad, delta, w, h, d.transform.rotation);
      const t = moved(d.transform, snap.delta, w, refW);
      return {
        set: { transform: t },
        summary: "move",
        quad: translate(d.quad, snap.delta),
        cropQuad: translate(d.cropQuad, snap.delta),
        lines: snap.lines,
      };
    }
    case "scale": {
      const from = d.cropQuad[d.handle.corner];
      const t = scaled(d.transform, c, from, add(from, delta));
      const k = t.scale / Math.max(d.transform.scale, 1e-6);
      return {
        set: { transform: t },
        summary: "scale",
        quad: turnScale(d.quad, c, 0, k),
        cropQuad: turnScale(d.cropQuad, c, 0, k),
        lines: { x: [], y: [] },
      };
    }
    case "rotate": {
      const t = rotated(d.transform, c, d.start, d.at, d.shift);
      const turn = t.rotation - d.transform.rotation;
      return {
        set: { transform: t },
        summary: "rotate",
        quad: turnScale(d.quad, c, turn, 1),
        cropQuad: turnScale(d.cropQuad, c, turn, 1),
        lines: { x: [], y: [] },
      };
    }
    case "crop": {
      const next = cropped(d.crop, d.quad, d.handle.side, delta);
      return {
        set: { crop: next },
        summary: "crop",
        quad: d.quad,
        cropQuad: cropQuad(d.quad, next),
        lines: { x: [], y: [] },
      };
    }
  }
}

const add = (a: Pt, b: Pt): Pt => [a[0] + b[0], a[1] + b[1]];
const points = (q: Quad) => q.map((p) => `${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(" ");

export function ViewerOverlay({
  project,
  time,
  playing,
  selection,
  width,
  height,
  onSelect,
  onEdit,
  onError,
}: {
  project: ProjectView_Serialize;
  /** Timeline time shown, seconds. */
  time: number;
  playing: boolean;
  /** Selected item ids, primary last. */
  selection: string[];
  /** The viewer rectangle in CSS pixels. */
  width: number;
  height: number;
  onSelect: (ids: string[]) => void;
  onEdit: (p: EditResult) => void;
  onError: (e: AppError) => void;
}) {
  const [geometry, setGeometry] = useState<ViewerGeometry | null>(null);
  const [fields, setFields] = useState<{ id: string; transform: Transform; crop: Crop } | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  /** The box a committed drag left, shown (and dragged from) until the new geometry arrives. */
  const [held, setHeld] = useState<{ itemId: string; quad: Quad; cropQuad: Quad } | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const preview = useRef<{ busy: boolean; next: { itemId: string; setJson: string } | null }>({ busy: false, next: null });
  const ref = useRef<HTMLDivElement>(null);
  const selected = selection[selection.length - 1] ?? null;
  const rev = project.rev;
  const [refW] = referenceSize(project.aspectRatio);

  // Geometry of the paused frame; none while playing (handles hide).
  useEffect(() => {
    if (playing) return;
    let live = true;
    fire(
      commands.viewerGeometry(time).then((r) => {
        if (!live) return;
        if (r.status === "ok") {
          setGeometry(r.data);
          setHeld(null);
        }
        else logError(r.error);
      }),
    );
    return () => {
      live = false;
    };
  }, [time, playing, rev]);

  // The selected item's transform and crop, the starting values of a drag.
  useEffect(() => {
    if (!selected) return;
    let live = true;
    fire(
      commands.itemDetail(selected).then((r) => {
        if (!live || r.status !== "ok") return;
        setFields({ id: selected, ...readFields(JSON.parse(r.data) as Record<string, unknown>) });
      }),
    );
    return () => {
      live = false;
    };
  }, [selected, rev]);

  const sendPreview = useCallback((p: { itemId: string; setJson: string } | null) => {
    const q = preview.current;
    if (p === null) {
      q.next = null;
      fire(commands.viewerPreview(null));
      return;
    }
    if (q.busy) {
      q.next = p;
      return;
    }
    q.busy = true;
    const run = (x: { itemId: string; setJson: string }) => {
      fire(
        commands.viewerPreview(x).then(() => {
          const n = q.next;
          q.next = null;
          if (n && dragRef.current) run(n);
          else q.busy = false;
        }),
      );
    };
    run(p);
  }, []);

  const cancel = useCallback(() => {
    const d = dragRef.current;
    dragRef.current = null;
    setDrag(null);
    if (d?.dragging) sendPreview(null);
  }, [sendPreview]);

  // Esc cancels a drag: the layer goes back and no revision is made.
  useEffect(() => {
    if (!drag) return;
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        cancel();
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [drag, cancel]);

  // Playback starting mid-drag cancels it.
  useEffect(() => {
    if (playing && dragRef.current) cancel();
  }, [playing, cancel]);

  const layer: ViewerLayer | undefined = playing ? undefined : geometry?.layers.find((l) => l.itemId === selected && l.selectable);
  const hold = held && held.itemId === selected ? held : null;
  const quad = layer ? (hold?.quad ?? toBox(layer.quad, width, height)) : null;
  const cq = layer ? (hold?.cropQuad ?? toBox(layer.cropQuad, width, height)) : null;
  const shows = !playing && !!layer && !!quad && !!cq && fields?.id === selected;
  const change = drag?.dragging ? dragChange(drag, width, height, refW) : null;
  const showQuad = change?.quad ?? quad;
  const showCrop = change?.cropQuad ?? cq;
  const can = handlesFor(layer?.itemType ?? "");

  const at = (e: React.PointerEvent): Pt => {
    const r = ref.current?.getBoundingClientRect();
    return [e.clientX - (r?.left ?? 0), e.clientY - (r?.top ?? 0)];
  };

  /** Starts a gesture; `clickOnly` presses (off the selected layer) can only become a click. */
  const begin = (e: React.PointerEvent, handle: Handle, clickOnly = false) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const p = at(e);
    const base = {
      start: p,
      at: p,
      shift: e.shiftKey,
      dragging: false,
    };
    const d: Drag =
      !clickOnly && shows && quad && cq && fields && selected
        ? { ...base, handle, itemId: selected, quad, cropQuad: cq, transform: fields.transform, crop: fields.crop }
        : {
            ...base,
            handle: { kind: "move" },
            itemId: "",
            quad: [p, p, p, p],
            cropQuad: [p, p, p, p],
            transform: NO_TRANSFORM,
            crop: NO_CROP,
          };
    dragRef.current = d;
    setDrag(d);
    try {
      ref.current?.setPointerCapture(e.pointerId);
    } catch {
      // Capture is best effort (a synthetic event has no active pointer).
    }
  };

  const onDown = (e: React.PointerEvent) => {
    const h = parseHandle((e.target as Element).getAttribute?.("data-handle"));
    if (h) {
      begin(e, h);
      return;
    }
    // On the selected layer a press may start a move; anywhere else it is a click.
    const onLayer = shows && cq ? contains(cq, at(e)) : false;
    begin(e, { kind: "move" }, !onLayer);
  };

  const onMove = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    const p = at(e);
    const travel = Math.hypot(p[0] - d.start[0], p[1] - d.start[1]);
    const next: Drag = { ...d, at: p, shift: e.shiftKey, dragging: d.dragging || (travel > CLICK_PX && !!d.itemId) };
    dragRef.current = next;
    setDrag(next);
    if (next.dragging) {
      const c = dragChange(next, width, height, refW);
      if (c) sendPreview({ itemId: next.itemId, setJson: JSON.stringify(c.set) });
    }
  };

  const onUp = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    dragRef.current = null;
    setDrag(null);
    const p = at(e);
    if (!d.dragging) {
      // A click: select what is drawn under the pointer (nothing there clears the selection).
      const fx = p[0] / Math.max(width, 1);
      const fy = p[1] / Math.max(height, 1);
      fire(
        commands.viewerPick(time, fx, fy).then((r) => {
          if (r.status === "ok") onSelect(r.data ? [r.data] : []);
          else onError(r.error);
        }),
      );
      return;
    }
    const c = dragChange({ ...d, at: p, shift: e.shiftKey }, width, height, refW);
    if (!c) return;
    // One revision for the whole gesture; the edit itself refreshes the viewer.
    preview.current.next = null;
    onEdit(updateItem(d.itemId, c.set, [], c.summary));
    setHeld({ itemId: d.itemId, quad: c.quad, cropQuad: c.cropQuad });
    setFields({ id: d.itemId, transform: (c.set.transform as Transform) ?? d.transform, crop: (c.set.crop as Crop) ?? d.crop });
  };

  const handle = (key: string, p: Pt, h: Handle, cursor: string, testId: string) => (
    <rect
      key={key}
      className="overlay-handle"
      data-testid={testId}
      x={p[0] - 5}
      y={p[1] - 5}
      width={10}
      height={10}
      style={{ cursor }}
      data-handle={handleName(h)}
    />
  );

  return (
    <div
      ref={ref}
      className="viewer-overlay"
      data-testid="viewer-overlay"
      data-playing={playing ? "true" : "false"}
      style={{ position: "absolute", inset: 0, zIndex: 2 }}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={cancel}
    >
      <svg width={width} height={height} style={{ position: "absolute", inset: 0, overflow: "visible" }}>
        {change?.lines.x.map((x) => (
          <line key={`sx${x}`} className="overlay-snap" x1={x} y1={0} x2={x} y2={height} data-testid="overlay-snap" />
        ))}
        {change?.lines.y.map((y) => (
          <line key={`sy${y}`} className="overlay-snap" x1={0} y1={y} x2={width} y2={y} data-testid="overlay-snap" />
        ))}
        {shows && showQuad && showCrop && (
          <g data-testid="overlay-handles" data-item={selected ?? ""}>
            {change?.summary === "crop" && <polygon className="overlay-source" points={points(showQuad)} />}
            <polygon className="overlay-box" data-testid="overlay-box" points={points(showCrop)} />
            {showCrop.map((p, i) => handle(`s${i}`, p, { kind: "scale", corner: i }, "nwse-resize", `handle-scale-${i}`))}
            {can.rotate && (
              <>
                <line
                  className="overlay-stem"
                  x1={(showCrop[0][0] + showCrop[1][0]) / 2}
                  y1={(showCrop[0][1] + showCrop[1][1]) / 2}
                  x2={rotateHandle(showCrop)[0]}
                  y2={rotateHandle(showCrop)[1]}
                />
                <circle
                  className="overlay-handle overlay-rotate"
                  data-testid="handle-rotate"
                  cx={rotateHandle(showCrop)[0]}
                  cy={rotateHandle(showCrop)[1]}
                  r={6}
                  style={{ cursor: "grab" }}
                  data-handle="rotate"
                />
              </>
            )}
            {can.crop &&
              (Object.entries(cropHandles(showCrop)) as [Side, Pt][]).map(([side, p]) => (
                <circle
                  key={side}
                  className="overlay-handle overlay-crop"
                  data-testid={`handle-crop-${side}`}
                  cx={p[0]}
                  cy={p[1]}
                  r={5}
                  style={{ cursor: "crosshair" }}
                  data-handle={`crop:${side}`}
                />
              ))}
          </g>
        )}
      </svg>
    </div>
  );
}


function handleName(h: Handle): string {
  if (h.kind === "scale") return `scale:${h.corner}`;
  if (h.kind === "crop") return `crop:${h.side}`;
  return h.kind;
}

function parseHandle(name: string | null | undefined): Handle | null {
  if (!name) return null;
  const [kind, arg] = name.split(":");
  if (kind === "rotate") return { kind: "rotate" };
  if (kind === "scale") return { kind: "scale", corner: Number(arg) || 0 };
  if (kind === "crop" && (arg === "top" || arg === "right" || arg === "bottom" || arg === "left")) return { kind: "crop", side: arg };
  return null;
}
