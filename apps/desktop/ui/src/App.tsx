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
import { ErrorBoundary } from "./ErrorBoundary";
import {
  ExportDialog,
  type PreviewMode,
  PreviewModeToggle,
  referenceSize,
} from "./ExportDialog";
import { Home } from "./Home";
import { fire, logError, runCommand, setErrorSink, unlisten } from "./notify";
import { EDIT_ACTIONS, keyToAction } from "./keys";
import {
  templateAspect,
  templateMessage,
  templateProjectName,
} from "./templates";

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

const CHAT_OPEN_KEY = "ove.chatOpen";

/** Whether the person last opened or closed the assistant (null: they never chose). */
function storedChatOpen(): boolean | null {
  try {
    const v = localStorage.getItem(CHAT_OPEN_KEY);
    return v === "1" ? true : v === "0" ? false : null;
  } catch {
    return null;
  }
}

function storeChatOpen(open: boolean) {
  try {
    localStorage.setItem(CHAT_OPEN_KEY, open ? "1" : "0");
  } catch {
    // Private windows and blocked storage: the choice lasts until the window closes.
  }
}

/** A project name from the first words of a prompt. */
function projectNameFrom(text: string): string {
  return (
    text
      .split(/\s+/)
      .slice(0, 6)
      .join(" ")
      .replace(/[^\p{L}\p{N} '-]/gu, "")
      .slice(0, 48)
      .trim() || "Untitled Project"
  );
}

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
  // The assistant opens by default only once a key is set (see the wiring effect) or when the person opened it.
  const [layout, setLayout] = useState<Layout>(() => ({
    panelOpen: true,
    timelineOpen: true,
    chatOpen: storedChatOpen() ?? false,
    viewerOpen: true,
  }));
  /** Layout changes from the editor; opening or closing the assistant is remembered. */
  const changeLayout = useCallback(
    (next: Layout) => {
      if (next.chatOpen !== layout.chatOpen) storeChatOpen(next.chatOpen);
      setLayout(next);
    },
    [layout.chatOpen],
  );
  const [mode, setMode] = useState<PreviewMode>("timeline");
  const [exporting, setExporting] = useState(false);
  const modeRef = useRef(mode);
  /** Frame rate of what the viewer plays (the timeline's while in timeline mode). */
  const fpsRef = useRef(30);
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
        runCommand(
          commands.transport({
            kind: "seekFrame",
            frame: Math.max(0, Math.round(at * (r.data.frameRate || 30))),
          }),
          undefined,
          showError,
        );
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
        runCommand(
          commands.transport({ kind: "seekFrame", frame }),
          undefined,
          showError,
        );
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
      runCommand(
        commands.transport({ kind: "seekFrame", frame }),
        undefined,
        showError,
      );
    },
    [showError],
  );

  const openProjectView = useCallback(
    (v: ProjectView_Serialize) => {
      setProject(v);
      projectRef.current = v;
      setRoute("editor");
      if (modeRef.current === "timeline") return fire(showTimeline(0));
      const first =
        v.items.find((i) => i.mediaId && i.trackId === "t_main") ??
        v.items.find((i) => i.mediaId);
      if (first) fire(preview(first));
      else {
        setMedia(null);
        runCommand(commands.closeMedia(), undefined, showError);
      }
    },
    [preview, showError, showTimeline],
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
    if (modeRef.current === "timeline") fire(showTimeline());
  }, [showError, showInfo, showTimeline]);

  const importInto = useCallback(
    async (paths: string[] | null) => {
      if (!projectRef.current) {
        const created = await commands.createProject(
          paths?.[0]
            ?.split(/[\\/]/)
            .pop()
            ?.replace(/\.[^.]+$/, "") ?? "Untitled Project",
          null,
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
        fire(showTimeline(added?.start ?? 0));
      } else if (added) fire(preview(added));
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
      if (m === "timeline") return fire(showTimeline(sel?.start ?? 0));
      const clip = sel?.mediaId
        ? sel
        : (p.items.find((i) => i.mediaId && i.trackId === "t_main") ??
          p.items.find((i) => i.mediaId));
      if (clip) fire(preview(clip));
      else {
        setMedia(null);
        runCommand(commands.closeMedia(), undefined, showError);
      }
    },
    [preview, showError, showTimeline],
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
      runCommand(commands.transport(command), undefined, showError);
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

  /** New project (optionally with an aspect ratio) whose first message goes to the assistant. */
  const startWithAssistant = async (
    text: string,
    name: string,
    aspect?: string,
    template?: string,
  ) => {
    const created = await commands.createProject(name, template ?? null);
    if (created.status === "error") return showError(created.error);
    let view = created.data;
    if (aspect && aspect !== view.aspectRatio) {
      const r = await commands.setAspectRatio(aspect);
      if (r.status === "ok") view = r.data;
      else showError(r.error);
    }
    const status = await commands.agentStatus();
    const ready = status.status === "ok" && status.data.hasKey;
    // Without a key the words wait in the assistant's composer next to the key card.
    if (!ready) setPendingDraft(text);
    setLayout((l) => ({ ...l, chatOpen: true }));
    openProjectView(view);
    if (ready) {
      const sent = await commands.agentSend(null, text);
      if (sent.status === "error") showError(sent.error);
    }
  };

  // Failures from work nobody awaits (see notify.ts) show as toasts.
  useEffect(() => setErrorSink(showError), [showError]);

  /** The assistant's `seek_viewer`: timeline seconds, so the viewer switches to the timeline if it shows a source. */
  const seekTimeline = useCallback(
    (seconds: number) => {
      if (modeRef.current !== "timeline") {
        setMode("timeline");
        modeRef.current = "timeline";
        fire(showTimeline(seconds));
        return;
      }
      runCommand(
        commands.transport({
          kind: "seekFrame",
          frame: Math.max(0, Math.round(seconds * fpsRef.current)),
        }),
        undefined,
        showError,
      );
    },
    [showError, showTimeline],
  );

  // Bridge wiring: app info, routed paths, playback, project and error events.
  useEffect(() => {
    const listeners = [
      events.playbackChanged.listen((e) => {
        fpsRef.current = e.payload.frameRate || 30;
        setPlayback(e.payload);
      }),
      events.appErrorRaised.listen((e) => showError(e.payload)),
      events.viewerBackendChanged.listen((e) => setBackend(e.payload)),
      events.projectChanged.listen((e) => {
        if (e.payload) setProject(e.payload);
      }),
      // `ove app open` / `ove project open`: show that project in the editor.
      events.remoteOpened.listen((e) => openProjectView(e.payload)),
      events.openPathsRequested.listen((e) => {
        if (e.payload.paths.length > 0) fire(importInto(e.payload.paths));
      }),
    ];
    runCommand(
      commands.appInfo(),
      (info) => {
        setBackend(info.viewerBackend);
        setPlatform(
          info.os.toLowerCase().startsWith("mac")
            ? "macos"
            : info.os.toLowerCase(),
        );
      },
      showError,
    );
    // A problem recorded last time: say so once (reports stay on this computer).
    commands
      .crashReports()
      .then((reports) => {
        if (reports.some((r) => !r.seen)) {
          showInfo(
            "OpenVideoEditor ran into a problem last time. A report was saved on this computer — see Settings → Privacy.",
          );
          runCommand(commands.markCrashReportsSeen(), undefined, logError);
        }
      })
      .catch(logError);
    runCommand(commands.listProjects(), setProjects, showError);
    // No choice yet: show the assistant to people who have set it up, not a key prompt to everyone else.
    if (storedChatOpen() === null)
      runCommand(
        commands.agentStatus(),
        (s) => setLayout((l) => ({ ...l, chatOpen: s.hasKey })),
        logError,
      );
    runCommand(
      commands.takePendingOpenPaths(),
      (paths) => {
        if (paths.length > 0) fire(importInto(paths));
      },
      showError,
    );
    return () => listeners.forEach(unlisten);
  }, [importInto, openProjectView, showError, showInfo]);

  // The command line asks what is selected and which viewer mode is on (`ove app context`).
  useEffect(() => {
    const t = setTimeout(
      () =>
        runCommand(
          commands.reportUiState({ selection, viewer: mode }),
          undefined,
          logError,
        ),
      150,
    );
    return () => clearTimeout(t);
  }, [selection, mode]);

  // Native viewer under a transparent web view needs a transparent page behind the viewer box.
  useEffect(() => {
    document.documentElement.dataset.viewer =
      backend === "stream" ? "stream" : "native";
    document.documentElement.dataset.platform = platform;
  }, [backend, platform]);

  const goHome = useCallback(async () => {
    const closed = await commands.closeProject();
    if (closed.status === "error") showError(closed.error);
    setProject(null);
    projectRef.current = null;
    setSelection([]);
    setMedia(null);
    setRoute("home");
    await refreshProjects();
  }, [refreshProjects, showError]);

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
      else if (action.type === "undo") fire(run(commands.undo()));
      else if (action.type === "redo") fire(run(commands.redo()));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [route, transport, run]);

  return (
    <div className="app">
      {route === "home" ? (
        <ErrorBoundary region="Home" className="home">
          <Home
            projects={projects}
            onError={showError}
            onOpenFile={() => fire(importInto(null))}
            onBlank={() =>
              runCommand(
                commands.createProject("Untitled Project", null),
                openProjectView,
                showError,
              )
            }
            onPrompt={(text) =>
              fire(startWithAssistant(text, projectNameFrom(text)))
            }
            onTemplate={(t, values) =>
              fire(
                startWithAssistant(
                  templateMessage(t, values),
                  templateProjectName(t, values),
                  templateAspect(t, values),
                  t.name,
                ),
              )
            }
            onOpenProject={(bundle) =>
              runCommand(
                commands.openProject(bundle),
                openProjectView,
                showError,
              )
            }
          />
        </ErrorBoundary>
      ) : (
        <Editor
          project={project}
          selection={selection}
          media={viewerMedia}
          playback={playback}
          backend={backend}
          diagnosticsOpen={diagnostics}
          layout={layout}
          setLayout={changeLayout}
          onTransport={transport}
          onImport={() => fire(importInto(null))}
          onImportTimeline={() => fire(importTimelineFile())}
          onError={showError}
          onHome={() => fire(goHome())}
          onSeek={seekTimeline}
          onSelect={(item, offset, mode) => {
            if (!item) setSelection([]);
            else if (mode === "toggle")
              setSelection((s) =>
                s.includes(item.id)
                  ? s.filter((x) => x !== item.id)
                  : [...s, item.id],
              );
            else fire(preview(item, offset));
          }}
          onSelectMany={setSelection}
          onEdit={(p) => fire(run(p))}
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
        <ErrorBoundary
          region="Export"
          className="modal-backdrop"
          onClose={() => setExporting(false)}
        >
          <ExportDialog
            projectName={project.name}
            aspect={project.aspectRatio}
            onClose={() => setExporting(false)}
            onError={showError}
          />
        </ErrorBoundary>
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
