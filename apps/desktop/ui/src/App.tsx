import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  AppError,
  ItemView_Serialize,
  MediaInfo_Serialize,
  PlaybackState,
  ProjectSummary_Serialize,
  ProjectView_Serialize,
  TransportCommand,
  ViewerBackend,
} from "./bindings";
import { commands, events } from "./bridge";
import { setPendingDraft } from "./ChatPanel";
import { Editor, type Layout } from "./Editor";
import {
  ExportDialog,
  type PreviewMode,
  PreviewModeToggle,
  referenceSize,
} from "./ExportDialog";
import { Home } from "./Home";
import { EDIT_ACTIONS, keyToAction } from "./keys";

const EMPTY: PlaybackState = {
  hasMedia: false,
  frame: 0,
  totalFrames: 0,
  frameRate: 30,
  playing: false,
  speed: 0,
  looping: false,
  volume: 1,
  muted: false,
  audioAvailable: false,
};

interface Toast {
  id: number;
  error: AppError;
  info?: boolean;
}

let toastId = 0;

export function App() {
  const [route, setRoute] = useState<"home" | "editor">("home");
  const [project, setProject] = useState<ProjectView_Serialize | null>(null);
  const [projects, setProjects] = useState<ProjectSummary_Serialize[]>([]);
  const [selection, setSelection] = useState<string[]>([]);
  const [media, setMedia] = useState<MediaInfo_Serialize | null>(null);
  const [playback, setPlayback] = useState<PlaybackState>(EMPTY);
  const [backend, setBackend] = useState<ViewerBackend>("stream");
  const [platform, setPlatform] = useState("");
  const [diagnostics, setDiagnostics] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [layout, setLayout] = useState<Layout>({
    panelOpen: true,
    timelineOpen: true,
    chatOpen: true,
    viewerOpen: true,
  });
  const [mode, setMode] = useState<PreviewMode>("timeline");
  const [exporting, setExporting] = useState(false);
  const modeRef = useRef(mode);
  const projectRef = useRef(project);
  const selectionRef = useRef(selection);
  useEffect(() => {
    projectRef.current = project;
    selectionRef.current = selection;
  }, [project, selection]);

  const showError = useCallback((error: AppError) => {
    const id = ++toastId;
    setToasts((t) => [...t.slice(-3), { id, error }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 6000);
  }, []);
  /** A confirmation toast (same place as errors, neutral colour). */
  const showInfo = useCallback((message: string) => {
    const id = ++toastId;
    setToasts((t) => [
      ...t.slice(-3),
      { id, error: { code: "internal", message }, info: true },
    ]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 6000);
  }, []);

  const refreshProjects = useCallback(async () => {
    const r = await commands.listProjects();
    if (r.status === "ok") setProjects(r.data);
    else showError(r.error);
  }, [showError]);

  /** Plays the open project's composited timeline in the viewer, optionally starting at a timeline time. */
  const showTimeline = useCallback(
    async (at?: number) => {
      const r = await commands.openTimeline();
      if (r.status === "error") return showError(r.error);
      setMedia(r.data);
      if (at != null)
        void commands.transport({
          kind: "seekFrame",
          frame: Math.max(0, Math.round(at * (r.data.frameRate || 30))),
        });
    },
    [showError],
  );

  /** Shows an item's source in the viewer, positioned at the item's in-point (+ an offset into the item). */
  const preview = useCallback(
    async (item: ItemView_Serialize, offsetSeconds = 0) => {
      const p = projectRef.current;
      const m = p?.media.find((x) => x.id === item.mediaId);
      setSelection([item.id]);
      if (modeRef.current === "timeline") {
        // The timeline viewer seeks to the clicked point instead of switching to the clip's source.
        const frame = Math.max(
          0,
          Math.round(((item.start ?? 0) + offsetSeconds) * 30),
        );
        void commands.transport({ kind: "seekFrame", frame });
        return;
      }
      if (!m) return;
      const r = await commands.openMedia(m.path);
      if (r.status === "error") return showError(r.error);
      setMedia(r.data);
      const fps = r.data.frameRate ?? 30;
      const frame = Math.max(
        0,
        Math.round(((item.trimStart ?? 0) + offsetSeconds) * fps),
      );
      void commands.transport({ kind: "seekFrame", frame });
    },
    [showError],
  );

  const openProjectView = useCallback(
    (v: ProjectView_Serialize) => {
      setProject(v);
      projectRef.current = v;
      setRoute("editor");
      if (modeRef.current === "timeline") return void showTimeline(0);
      const first =
        v.items.find((i) => i.mediaId && i.trackId === "t_main") ??
        v.items.find((i) => i.mediaId);
      if (first) void preview(first);
      else {
        setMedia(null);
        void commands.closeMedia();
      }
    },
    [preview, showTimeline],
  );

  /** Adds a timeline file (FCPXML, XML, EDL, OTIO) to the open project. */
  const importTimelineFile = useCallback(async () => {
    const r = await commands.importTimeline(null, 30);
    if (r.status === "error") return showError(r.error);
    if (!r.data) return;
    setProject(r.data.project);
    projectRef.current = r.data.project;
    const { clips, tracks, mediaAdded, missing } = r.data;
    showInfo(
      `Imported ${clips} clip${clips === 1 ? "" : "s"} on ${tracks} track${tracks === 1 ? "" : "s"}` +
        (mediaAdded
          ? `, ${mediaAdded} new media file${mediaAdded === 1 ? "" : "s"}`
          : "") +
        (missing.length
          ? `. Missing media (placeholders): ${missing.slice(0, 3).join(", ")}${missing.length > 3 ? "…" : ""}`
          : "."),
    );
    if (modeRef.current === "timeline") void showTimeline();
  }, [showError, showInfo, showTimeline]);

  const importInto = useCallback(
    async (paths: string[] | null) => {
      if (!projectRef.current) {
        const created = await commands.createProject(
          paths?.[0]
            ?.split(/[\\/]/)
            .pop()
            ?.replace(/\.[^.]+$/, "") ?? "Untitled Project",
        );
        if (created.status === "error") return showError(created.error);
        setProject(created.data);
        projectRef.current = created.data;
        setRoute("editor");
      }
      const before = new Set(projectRef.current?.items.map((i) => i.id));
      const r = paths
        ? await commands.importMedia(paths)
        : await commands.importMediaDialog();
      if (r.status === "error") return showError(r.error);
      if (!r.data) return;
      setProject(r.data);
      projectRef.current = r.data;
      const added = r.data.items.find((i) => !before.has(i.id) && i.mediaId);
      if (modeRef.current === "timeline") {
        if (added) setSelection([added.id]);
        void showTimeline(added?.start ?? 0);
      } else if (added) void preview(added);
    },
    [preview, showError, showTimeline],
  );

  const switchMode = useCallback(
    (m: PreviewMode) => {
      if (m === modeRef.current) return;
      setMode(m);
      modeRef.current = m;
      const p = projectRef.current;
      if (!p) return;
      const sel = p.items.find(
        (i) => i.id === selectionRef.current[selectionRef.current.length - 1],
      );
      if (m === "timeline") return void showTimeline(sel?.start ?? 0);
      const clip = sel?.mediaId
        ? sel
        : (p.items.find((i) => i.mediaId && i.trackId === "t_main") ??
          p.items.find((i) => i.mediaId));
      if (clip) void preview(clip);
      else {
        setMedia(null);
        void commands.closeMedia();
      }
    },
    [preview, showTimeline],
  );

  // The engine's timeline follows edits; keep the viewer's frame shape in step with the aspect ratio.
  const aspect = project?.aspectRatio;
  const viewerMedia = useMemo(() => {
    if (!aspect || media?.videoCodec !== "timeline") return media;
    const [width, height] = referenceSize(aspect);
    return media.width === width && media.height === height
      ? media
      : { ...media, width, height };
  }, [media, aspect]);

  const transport = useCallback(
    (command: TransportCommand) => {
      // The engine applies commands asynchronously and reports every change through `playbackChanged`;
      // replies are only checked for errors so a late reply never overwrites a newer state.
      void commands.transport(command).then((r) => {
        if (r.status === "error") showError(r.error);
      });
    },
    [showError],
  );

  const run = useCallback(
    async (
      p: Promise<
        | { status: "ok"; data: ProjectView_Serialize }
        | { status: "error"; error: AppError }
      >,
    ) => {
      const r = await p;
      if (r.status === "error") showError(r.error);
      else setProject(r.data);
    },
    [showError],
  );

  // Bridge wiring: app info, routed paths, playback, project and error events.
  useEffect(() => {
    const unlisten = [
      events.playbackChanged.listen((e) => setPlayback(e.payload)),
      events.appErrorRaised.listen((e) => showError(e.payload)),
      events.viewerBackendChanged.listen((e) => setBackend(e.payload)),
      events.projectChanged.listen((e) => {
        if (e.payload) setProject(e.payload as ProjectView_Serialize);
      }),
      events.openPathsRequested.listen((e) => {
        if (e.payload.paths.length > 0) void importInto(e.payload.paths);
      }),
    ];
    void commands.appInfo().then((r) => {
      if (r.status === "ok") {
        setBackend(r.data.viewerBackend);
        setPlatform(
          r.data.os.toLowerCase().startsWith("mac")
            ? "macos"
            : r.data.os.toLowerCase(),
        );
      }
    });
    void commands.listProjects().then((r) => {
      if (r.status === "ok") setProjects(r.data);
    });
    void commands.takePendingOpenPaths().then((r) => {
      if (r.status === "ok" && r.data.length > 0) void importInto(r.data);
    });
    return () => unlisten.forEach((p) => void p.then((f) => f()));
  }, [importInto, showError]);

  // Native viewer under a transparent web view needs a transparent page behind the viewer box.
  useEffect(() => {
    document.documentElement.dataset.viewer =
      backend === "stream" ? "stream" : "native";
    document.documentElement.dataset.platform = platform;
  }, [backend, platform]);

  const goHome = useCallback(async () => {
    await commands.closeProject();
    setProject(null);
    projectRef.current = null;
    setSelection([]);
    setMedia(null);
    setRoute("home");
    void refreshProjects();
  }, [refreshProjects]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = keyToAction(e);
      if (!action) return;
      if (action.type === "toggleDiagnostics") {
        e.preventDefault();
        setDiagnostics((d) => !d);
        return;
      }
      // Editing keys (delete, split, copy/paste, zoom, …) are handled by the editor.
      if (route !== "editor" || EDIT_ACTIONS.has(action.type)) return;
      e.preventDefault();
      if (action.type === "transport") transport(action.command);
      else if (action.type === "undo") void run(commands.undo());
      else if (action.type === "redo") void run(commands.redo());
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [route, transport, run]);

  return (
    <div className="app">
      {route === "home" ? (
        <Home
          projects={projects}
          onOpenFile={() => void importInto(null)}
          onBlank={() =>
            void commands.createProject("Untitled Project").then((r) => {
              if (r.status === "error") showError(r.error);
              else openProjectView(r.data);
            })
          }
          onPrompt={(text) =>
            void (async () => {
              const name =
                text
                  .split(/\s+/)
                  .slice(0, 6)
                  .join(" ")
                  .replace(/[^\p{L}\p{N} '-]/gu, "")
                  .slice(0, 48)
                  .trim() || "Untitled Project";
              const created = await commands.createProject(name);
              if (created.status === "error") return showError(created.error);
              const status = await commands.agentStatus();
              const ready = status.status === "ok" && status.data.hasKey;
              // Without a key the words wait in the assistant's composer next to the key card.
              if (!ready) setPendingDraft(text);
              setLayout((l) => ({ ...l, chatOpen: true }));
              openProjectView(created.data);
              if (ready) {
                const sent = await commands.agentSend(null, text);
                if (sent.status === "error") showError(sent.error);
              }
            })()
          }
          onOpenProject={(bundle) =>
            void commands.openProject(bundle).then((r) => {
              if (r.status === "error") showError(r.error);
              else openProjectView(r.data);
            })
          }
        />
      ) : (
        <Editor
          project={project}
          selection={selection}
          media={viewerMedia}
          playback={playback}
          backend={backend}
          diagnosticsOpen={diagnostics}
          layout={layout}
          setLayout={setLayout}
          onTransport={transport}
          onImport={() => void importInto(null)}
          onImportTimeline={() => void importTimelineFile()}
          onError={showError}
          onHome={() => void goHome()}
          onSelect={(item, offset, mode) => {
            if (!item) setSelection([]);
            else if (mode === "toggle")
              setSelection((s) =>
                s.includes(item.id)
                  ? s.filter((x) => x !== item.id)
                  : [...s, item.id],
              );
            else void preview(item, offset);
          }}
          onSelectMany={setSelection}
          onEdit={(p) => void run(p)}
          timelineMode={mode === "timeline"}
          barExtras={
            <>
              <PreviewModeToggle mode={mode} onChange={switchMode} />
              <button
                className="btn primary xp-open"
                onClick={() => setExporting(true)}
                data-testid="open-export"
              >
                Export
              </button>
            </>
          }
        />
      )}
      {exporting && project && (
        <ExportDialog
          projectName={project.name}
          aspect={project.aspectRatio}
          onClose={() => setExporting(false)}
          onError={showError}
        />
      )}
      <div className="toasts" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={t.info ? "toast info" : "toast"}
            data-testid="toast"
            data-code={t.info ? "info" : t.error.code}
          >
            {t.error.message}
            <button
              className="icon-btn"
              aria-label="Dismiss"
              onClick={() => setToasts((x) => x.filter((y) => y.id !== t.id))}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
