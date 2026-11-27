// A clip's sound on the timeline: its waveform (from the import analysis cache, drawn for the visible part of the
// clip only) and its gain line. Drag the line to change the clip's volume; Alt-click (Option-click on macOS) adds a
// volume keyframe; drag a point to move it in time and level; Delete removes the selected point. Each gesture is one
// revision, committed on release.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ItemView_Serialize, PeakRangeView } from "./bindings";
import "./audio.css";
import { events } from "./bridge";
import type { EditResult } from "./edit";
import { updateItem } from "./edit";
import {
  type VolumeKey,
  clearPeaks,
  clipAudio,
  dbToGain,
  drawWaveform,
  gainAt,
  gainToY,
  keyDbAt,
  lineGainAt,
  loadPeaks,
  peakKey,
  setVolumeKeys,
  sourceRange,
  yToPercent,
} from "./waveform";

/** Canvas pixels drawn beyond each side of the visible area, so short scrolls need no redraw request. */
const MARGIN_PX = 200;
/** Widest canvas drawn at once. */
const MAX_CANVAS = 4096;
/** Points of the gain line between keyframes (the curve is linear in dB, not in percent). */
const CURVE_STEPS = 48;

let analysisVersion = 0;
const versionListeners = new Set<() => void>();
let subscribed = false;

/** One app-wide listener: an analysis finished, so every clip asks for its peaks again. */
function useAnalysisVersion(): number {
  const [v, setV] = useState(analysisVersion);
  useEffect(() => {
    const on = () => setV(analysisVersion);
    versionListeners.add(on);
    if (!subscribed) {
      subscribed = true;
      events.audioAnalysisChanged
        .listen(() => {
          clearPeaks();
          analysisVersion++;
          for (const l of versionListeners) l();
        })
        .catch(() => {
          subscribed = false;
        });
    }
    return () => {
      versionListeners.delete(on);
    };
  }, []);
  return v;
}

/** The visible part of an element inside its horizontal scroller, in the element's own pixels. */
function visibleSpan(el: HTMLElement | null, width: number): [number, number] {
  const scroller = el?.closest<HTMLElement>(".tl-scroll");
  if (!el || !scroller) return [0, Math.min(width, MAX_CANVAS)];
  const r = el.getBoundingClientRect();
  const s = scroller.getBoundingClientRect();
  if (!s.width) return [0, Math.min(width, MAX_CANVAS)]; // No layout (tests): the start of the clip.
  const a = Math.max(0, Math.floor(s.left - r.left - MARGIN_PX));
  const b = Math.min(width, Math.ceil(s.right - r.left + MARGIN_PX));
  return b <= a ? [0, 0] : [a, Math.min(b, a + MAX_CANVAS)];
}

export interface ClipAudioLayerProps {
  item: ItemView_Serialize;
  /** Clip width and height in pixels. */
  width: number;
  height: number;
  onEdit: (p: EditResult) => void;
}

type GainDrag =
  | { kind: "line"; startY: number; startVolume: number; volume: number }
  | { kind: "point"; index: number; startX: number; startY: number; key: VolumeKey; moved: boolean };

export function ClipAudioLayer({ item, width, height, onEdit }: ClipAudioLayerProps) {
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const version = useAnalysisVersion();
  const [span, setSpan] = useState<[number, number]>([0, Math.min(width, MAX_CANVAS)]);
  const [loaded, setLoaded] = useState<{ key: string; view: PeakRangeView | null } | null>(null);
  const [drag, setDrag] = useState<GainDrag | null>(null);
  const [selectedKey, setSelectedKey] = useState<number | null>(null);
  const base = useMemo(() => clipAudio(item), [item]);
  const len = Math.max(1e-6, (item.end ?? 0) - (item.start ?? 0));

  // What the person sees during a drag: the dragged volume or point, before it is committed.
  const audio = useMemo(() => {
    if (drag?.kind === "line") return { ...base, volume: drag.volume };
    if (drag?.kind === "point") {
      const keys = base.volumeKeys.map((k, i) => (i === drag.index ? drag.key : k)).sort((a, b) => a.time - b.time);
      return { ...base, volumeKeys: keys };
    }
    return base;
  }, [base, drag]);

  // Track the visible part of the clip while the timeline scrolls or resizes.
  useLayoutEffect(() => {
    const update = () => {
      const next = visibleSpan(root.current, width);
      setSpan((s) => (Math.abs(s[0] - next[0]) > MARGIN_PX / 2 || Math.abs(s[1] - next[1]) > MARGIN_PX / 2 || next[1] - next[0] !== s[1] - s[0] ? next : s));
    };
    update();
    const scroller = root.current?.closest(".tl-scroll");
    scroller?.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", update);
    return () => {
      scroller?.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
    };
  }, [width]);

  // Ask for the peaks of the visible part's source range; gain is not part of the request.
  const src = sourceRange(item);
  const [x0, x1] = span;
  const cols = Math.max(0, x1 - x0);
  const frac = (x: number) => x / Math.max(1, width);
  const sA = src.from + (src.to - src.from) * frac(x0);
  const sB = src.from + (src.to - src.from) * frac(x1);
  const [lo, hi] = sA <= sB ? [sA, sB] : [sB, sA];
  const want = item.mediaId && cols >= 1 && hi > lo ? peakKey(item.mediaId, lo, hi, Math.round(cols)) : null;
  useEffect(() => {
    if (!want || !item.mediaId) return;
    let live = true;
    loadPeaks(item.mediaId, lo, hi, Math.round(cols)).then(
      (p) => {
        if (live) setLoaded({ key: want, view: p });
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [want, item.mediaId, lo, hi, cols, version]);
  // Peaks of an earlier range are not drawn for this one.
  const peaks = loaded && loaded.key === want ? loaded.view : null;

  // Draw: the visible columns, scaled by the heard gain at each clip time.
  useEffect(() => {
    const c = canvas.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx || !peaks || peaks.status !== "ready") return;
    drawWaveform(ctx, peaks, src.reverse, cols, height, (x) => gainAt(audio, ((x0 + x + 0.5) / Math.max(1, width)) * len, len));
  }, [peaks, audio, cols, height, x0, width, len, src.reverse]);

  // Gain-line geometry.
  const yOf = (rel: number) => gainToY(lineGainAt(audio, rel), height);
  const linePath = (() => {
    const keys = audio.volumeKeys;
    if (!keys.length) return `M0 ${yOf(0)} H${width}`;
    const pts: string[] = [];
    const n = Math.min(400, Math.max(CURVE_STEPS, Math.round(width / 6)));
    for (let s = 0; s <= n; s++) {
      const rel = (s / n) * len;
      pts.push(`${((rel / len) * width).toFixed(1)} ${yOf(rel).toFixed(1)}`);
    }
    return `M${pts.join(" L")}`;
  })();
  const shown = drag?.kind === "line" ? drag.volume : drag?.kind === "point" ? drag.key.value : null;

  const relAt = (clientX: number) => {
    const r = root.current?.getBoundingClientRect();
    const px = r ? clientX - r.left : 0;
    return Math.min(len, Math.max(0, (px / Math.max(1, width)) * len));
  };
  /** Keyframe percent for a line y, given the static volume (the line shows static × keyframe). */
  const keyPercentForY = (y: number) => {
    const eff = yToPercent(y, height);
    const v = base.volume > 0 ? (eff / base.volume) * 100 : eff;
    return Math.round(Math.min(200, Math.max(0, v)) * 10) / 10;
  };

  const onLineDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    if (e.altKey) {
      // Add a keyframe at the clicked time, at the level the line has there.
      const rel = relAt(e.clientX);
      const k = keyDbAt(base.volumeKeys, base.volumeEasing, rel);
      const value = Math.round((k === null ? 100 : dbToGain(k) * 100) * 10) / 10;
      const keys = [...base.volumeKeys, { time: rel, value }];
      onEdit(setVolumeKeys(item.id, keys, base.volumeEasing, "add volume keyframe"));
      return;
    }
    setSelectedKey(null);
    setDrag({ kind: "line", startY: e.clientY, startVolume: base.volume, volume: base.volume });
  };

  const onPointDown = (e: React.PointerEvent, index: number) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    setSelectedKey(index);
    (e.currentTarget as SVGElement).closest("svg")?.focus();
    setDrag({ kind: "point", index, startX: e.clientX, startY: e.clientY, key: base.volumeKeys[index], moved: false });
  };

  useEffect(() => {
    if (!drag) return;
    const move = (ev: PointerEvent) => {
      setDrag((d) => {
        if (!d) return d;
        if (d.kind === "line") {
          // Moving down lowers the volume; the line's y maps linearly to 0–200 %.
          const y0 = gainToY(d.startVolume / 100, height);
          return { ...d, volume: yToPercent(y0 + (ev.clientY - d.startY), height) };
        }
        const orig = base.volumeKeys[d.index];
        const dt = ((ev.clientX - d.startX) / Math.max(1, width)) * len;
        const prev = base.volumeKeys[d.index - 1]?.time ?? 0;
        const next = base.volumeKeys[d.index + 1]?.time ?? len;
        const time = Math.min(next, Math.max(prev, orig.time + dt));
        const y = gainToY(lineGainAt(base, orig.time), height) + (ev.clientY - d.startY);
        return { ...d, moved: true, key: { time, value: keyPercentForY(y) } };
      });
    };
    const up = () => {
      setDrag((d) => {
        if (d?.kind === "line" && d.volume !== d.startVolume) onEdit(updateItem(item.id, { volume: d.volume }, [], "volume"));
        if (d?.kind === "point" && d.moved) {
          const keys = base.volumeKeys.map((k, i) => (i === d.index ? d.key : k));
          onEdit(setVolumeKeys(item.id, keys, base.volumeEasing, "move volume keyframe"));
        }
        return null;
      });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    // The gesture keeps the values it started with; edits arrive as a new item after release.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag !== null]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.key === "Delete" || e.key === "Backspace") && selectedKey !== null && base.volumeKeys[selectedKey]) {
      e.stopPropagation();
      e.preventDefault();
      const keys = base.volumeKeys.filter((_, i) => i !== selectedKey);
      setSelectedKey(null);
      onEdit(setVolumeKeys(item.id, keys, base.volumeEasing, "delete volume keyframe"));
    }
  };

  const status = peaks?.status ?? (item.mediaId ? "pending" : "none");
  return (
    <div className="clip-audio" ref={root} data-testid="clip-audio" data-wave={status}>
      {status === "pending" && <span className="wave-placeholder" data-testid="wave-placeholder" />}
      {status === "error" && <span className="wave-error" data-testid="wave-error" title={peaks?.message ?? "No waveform"} />}
      {status === "ready" && (
        <canvas
          ref={canvas}
          className="wave"
          data-testid="waveform"
          data-cols={cols}
          data-source={`${lo.toFixed(3)}-${hi.toFixed(3)}`}
          data-reverse={src.reverse ? "1" : "0"}
          width={cols}
          height={height}
          style={{ left: x0, width: cols, height }}
        />
      )}
      <svg
        className={`gain-line${drag ? " dragging" : ""}`}
        width={width}
        height={height}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        data-testid="gain-line"
        data-volume={audio.volume}
        data-keys={audio.volumeKeys.length}
      >
        <path d={linePath} className="gain-path" />
        <path d={linePath} className="gain-hit" data-testid="gain-hit" onPointerDown={onLineDown} />
        {audio.volumeKeys.map((k, i) => (
          <circle
            key={i}
            className={`gain-point${selectedKey === i ? " selected" : ""}`}
            data-testid="gain-point"
            cx={(k.time / len) * width}
            cy={yOf(k.time)}
            r={4}
            onPointerDown={(e) => onPointDown(e, i)}
          />
        ))}
      </svg>
      {shown !== null && (
        <span className="gain-readout" data-testid="gain-readout">
          {Math.round(shown)} %
        </span>
      )}
    </div>
  );
}
