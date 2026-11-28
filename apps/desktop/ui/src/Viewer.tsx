import { KeySamplerOverlay } from "./KeySampler";
import { ViewerOverlay } from "./overlay/ViewerOverlay";
import "./overlay/overlay.css";
import { FullscreenControls } from "./overlay/FullscreenControls";
import { GuideLayer, GuidesMenu } from "./overlay/GuideLayer";
import { type Guides, type Preset, loadGuides, loadPresets, saveGuides, savePresets } from "./overlay/guides";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type {
  Diagnostics_Serialize,
  MediaInfo_Serialize,
  PlaybackState,
  TransportCommand,
  ViewerBackend,
  ViewerRect,
} from "./bindings";
import { commands, events, frameUrl, isTauri, type Result } from "./bridge";
import { COMING_LATER, ContextMenu, Dropdown, type MenuItem } from "./Menu";
import { useViewerRectSync } from "./viewerRect";
import { Icon } from "./icons";
import { fire, logError, runCommand, unlisten } from "./notify";
import type { AppError } from "./bindings";

const PADDING = 24;
const ZOOMS = [
  { id: "fit", label: "Fit", scale: 1 },
  { id: "75", label: "75%", scale: 0.75 },
  { id: "50", label: "50%", scale: 0.5 },
];

/** Aspect-fits the viewer box inside the area (CSS pixels). */
export function fitBox(areaW: number, areaH: number, aspect: number, scale: number) {
  const w = Math.max(0, areaW - PADDING * 2);
  const h = Math.max(0, areaH - PADDING * 2);
  let bw = w;
  let bh = w / aspect;
  if (bh > h) {
    bh = h;
    bw = h * aspect;
  }
  bw = Math.floor(bw * scale);
  bh = Math.floor(bh * scale);
  return { left: Math.round((areaW - bw) / 2), top: Math.round((areaH - bh) / 2), width: bw, height: bh };
}

/** Browser-only stand-in for streamed frames: bars plus a moving marker. */
function paintMockFrame(canvas: HTMLCanvasElement, frame: number, total: number) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const w = (canvas.width = 640);
  const h = (canvas.height = 360);
  const bars = ["#bfbfbf", "#bfbf00", "#00bfbf", "#00bf00", "#bf00bf", "#bf0000", "#0000bf", "#101010"];
  bars.forEach((c, i) => {
    ctx.fillStyle = c;
    ctx.fillRect((i * w) / 8, 0, w / 8 + 1, h * 0.75);
  });
  const grad = ctx.createLinearGradient(0, 0, w, 0);
  grad.addColorStop(0, "#000");
  grad.addColorStop(1, "#fff");
  ctx.fillStyle = grad;
  ctx.fillRect(0, h * 0.75, w, h * 0.25);
  if (total > 0) {
    ctx.fillStyle = "#fff";
    ctx.fillRect((frame / total) * w - 1, 0, 3, h);
  }
}

export function Viewer({
  media,
  frame,
  backend,
  visible,
  diagnosticsOpen,
  onError,
  toolbarLeft,
  overlay,
  transport,
}: {
  media: MediaInfo_Serialize | null;
  frame: number;
  backend: ViewerBackend;
  visible: boolean;
  diagnosticsOpen: boolean;
  onError: (e: AppError) => void;
  toolbarLeft?: React.ReactNode;
  /** Click to select and handles over the composited timeline (absent: the viewer shows a single file). */
  overlay?: Omit<React.ComponentProps<typeof ViewerOverlay>, "width" | "height" | "onError">;
  /** Playback state and commands for the full-screen controls. */
  transport?: { playback: PlaybackState; onTransport: (c: TransportCommand) => void };
}) {
  const projectId = overlay?.project.id ?? null;
  // Guides are per project view state, kept in this browser (never in the document).
  const [guideState, setGuideState] = useState<{ id: string | null; guides: Guides }>(() => ({
    id: projectId,
    guides: loadGuides(projectId),
  }));
  const guides = guideState.id === projectId ? guideState.guides : loadGuides(projectId);
  const setGuides = (g: Guides) => {
    saveGuides(projectId, g);
    setGuideState({ id: projectId, guides: g });
  };
  const [presets, setPresetList] = useState<Preset[]>(loadPresets);
  const setPresets = (p: Preset[]) => {
    savePresets(p);
    setPresetList(p);
  };
  const [full, setFull] = useState(false);
  const [activity, setActivity] = useState(0);
  const setFullscreen = useCallback(
    (on: boolean) => {
      setFull(on);
      runCommand(commands.viewerFullscreen(on), undefined, onError);
    },
    [onError],
  );
  // Esc leaves full screen (a handle drag takes Esc first and cancels itself).
  useEffect(() => {
    if (!full) return;
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") setFullscreen(false);
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [full, setFullscreen]);
  const areaRef = useRef<HTMLElement>(null);
  const rectRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [area, setArea] = useState({ w: 0, h: 0 });
  const [zoom, setZoom] = useState(ZOOMS[0]);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const stream = backend === "stream";

  useLayoutEffect(() => {
    const el = areaRef.current;
    if (!el) return;
    const update = () => setArea({ w: el.clientWidth, h: el.clientHeight });
    update();
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(update) : null;
    ro?.observe(el);
    return () => ro?.disconnect();
  }, [visible]);

  const send = useCallback(
    (rect: ViewerRect) => {
      runCommand(commands.setViewerRect(rect), undefined, onError);
    },
    [onError],
  );
  useViewerRectSync(rectRef, visible, send);

  // Candidate 3: paint streamed frames.
  useEffect(() => {
    if (!stream || !isTauri()) return;
    let busy = false;
    let again = false;
    const pull = async () => {
      if (busy) {
        again = true;
        return;
      }
      busy = true;
      try {
        do {
          again = false;
          const res = await fetch(frameUrl(), { cache: "no-store" });
          if (res.status !== 200) break;
          const w = Number(res.headers.get("X-Frame-Width"));
          const h = Number(res.headers.get("X-Frame-Height"));
          const buf = new Uint8ClampedArray(await res.arrayBuffer());
          const canvas = canvasRef.current;
          if (canvas && w > 0 && h > 0 && buf.length === w * h * 4) {
            if (canvas.width !== w) canvas.width = w;
            if (canvas.height !== h) canvas.height = h;
            canvas.getContext("2d")?.putImageData(new ImageData(buf, w, h), 0, 0);
          }
        } while (again);
      } catch {
        // A dropped streamed frame is not an error; the next one replaces it.
      } finally {
        busy = false;
      }
    };
    // `pull` catches its own failures (a dropped frame is replaced by the next one).
    const un = events.viewerFrameReady.listen(() => fire(pull()));
    fire(pull());
    return () => unlisten(un);
  }, [stream]);

  // Browser development: draw a local pattern instead of streamed frames.
  useEffect(() => {
    if (stream && !isTauri() && canvasRef.current) {
      paintMockFrame(canvasRef.current, frame, media?.totalFrames ?? 0);
    }
  }, [stream, frame, media]);

  const aspect = media && media.width > 0 && media.height > 0 ? media.width / media.height : 16 / 9;
  const box = fitBox(area.w, area.h, aspect, full ? 1 : zoom.scale);

  const contextItems: MenuItem[] = [
    { id: "fit", label: "Zoom to fit", onSelect: () => setZoom(ZOOMS[0]) },
    { id: "half", label: "Zoom 50%", onSelect: () => setZoom(ZOOMS[2]) },
    { id: "snapshot", label: "Copy frame", title: COMING_LATER },
  ];

  if (!visible) return null;
  return (
    <section
      className={`viewer-area ${stream ? "is-stream" : "is-native"}${full ? " is-full" : ""}`}
      ref={areaRef}
      data-testid="viewer-area"
      data-backend={backend}
      data-full={full ? "true" : "false"}
      onPointerMove={full ? () => setActivity((n) => n + 1) : undefined}
    >
      <div
        className="viewer-rect"
        ref={rectRef}
        data-testid="viewer-rect"
        style={{ left: box.left, top: box.top, width: box.width, height: box.height }}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenuAt({ x: e.clientX, y: e.clientY });
        }}
      >
        {stream && <canvas ref={canvasRef} className="viewer-canvas" data-testid="viewer-canvas" />}
        {!media && <div className="viewer-empty">Import media to see it here.</div>}
        <GuideLayer guides={guides} presets={presets} width={box.width} height={box.height} />
        {overlay && media && <ViewerOverlay {...overlay} width={box.width} height={box.height} onError={onError} />}
        <KeySamplerOverlay onError={onError} />
      </div>
      {toolbarLeft && !full && <div className="viewer-tools-left">{toolbarLeft}</div>}
      {full && transport && (
        <FullscreenControls
          playback={transport.playback}
          onTransport={transport.onTransport}
          onExit={() => setFullscreen(false)}
          activity={activity}
          guidesMenu={<GuidesMenu guides={guides} presets={presets} onChange={setGuides} onPresets={setPresets} testId="fs-guides" />}
        />
      )}
      <div className="viewer-tools" hidden={full}>
        <GuidesMenu guides={guides} presets={presets} onChange={setGuides} onPresets={setPresets} />
        <button
          className="pill"
          data-testid="viewer-fullscreen"
          title="Full screen (Esc leaves)"
          onClick={() => setFullscreen(true)}
        >
          <Icon name="expand" size={14} />
        </button>
        <Dropdown
          testId="viewer-zoom"
          label={zoom.label}
          items={ZOOMS.map((z) => ({ id: z.id, label: z.label, onSelect: () => setZoom(z) }))}
        />
      </div>
      {diagnosticsOpen && <DiagnosticsOverlay onError={onError} />}
      <ContextMenu at={menuAt} items={contextItems} onClose={() => setMenuAt(null)} />
    </section>
  );
}

function fmtBytes(n: number | null) {
  return n == null ? "n/a" : `${(n / (1024 * 1024)).toFixed(0)} MB`;
}

export function DiagnosticsOverlay({ onError }: { onError: (e: AppError) => void }) {
  const [d, setD] = useState<Diagnostics_Serialize | null>(null);
  useEffect(() => {
    let live = true;
    // Polled twice a second: a failure is logged, not shown (the overlay would keep raising it).
    const poll = () =>
      runCommand(
        commands.diagnostics(),
        (data) => {
          if (live) setD(data);
        },
        logError,
      );
    poll();
    const t = setInterval(poll, 500);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, []);
  const v = d?.viewer;
  const run = (p: Promise<Result<null>>) => runCommand(p, undefined, onError);
  return (
    <div className="diagnostics" data-testid="diagnostics">
      <div>decode: {d?.decodePath ?? "…"}</div>
      <div>presenter: {v?.backend ?? "…"} · GPU: {v?.gpuAdapter ?? "none"}</div>
      <div>
        frame: {v ? `${v.frameWidth}×${v.frameHeight} ${v.pixelFormat ?? ""}` : "…"} · copies: {v?.copiesLastFrame ?? 0}
      </div>
      <div>
        surface: {v ? `${v.physicalWidth}×${v.physicalHeight}px` : "…"} · image {v ? `${v.imageWidth}×${v.imageHeight}` : ""}
      </div>
      <div>
        presented: {v?.presentedFrames ?? 0} · dropped: {d?.droppedFrames ?? 0} · recoveries: {v?.deviceRecoveries ?? 0}
      </div>
      <div>
        A/V offset: {d?.avOffsetMs == null ? "n/a" : `${d.avOffsetMs.toFixed(1)} ms`} · seek:{" "}
        {v?.lastSeekMs == null ? "n/a" : `${v.lastSeekMs.toFixed(1)} ms`} · present: {(v?.lastPresentMs ?? 0).toFixed(1)} ms
      </div>
      <div>memory: {fmtBytes(d?.processMemoryBytes ?? null)} · audio: {d?.audioAvailable ? "on" : "unavailable"}</div>
      <div className="diagnostics-actions">
        <button onClick={() => run(commands.simulateDeviceLoss())}>Simulate GPU loss</button>
        <button onClick={() => run(commands.debugPanicWorker())}>Panic a worker</button>
        <button onClick={() => run(commands.debugPanicViewer())}>Panic the viewer</button>
      </div>
    </div>
  );
}
