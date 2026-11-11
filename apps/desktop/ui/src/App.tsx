import { useCallback, useEffect, useState } from "react";
import type { AppError, MediaInfo_Serialize, PlaybackState, TransportCommand, ViewerBackend } from "./bindings";
import { commands, events } from "./bridge";
import { Editor, type Layout } from "./Editor";
import { Home } from "./Home";
import { keyToAction } from "./keys";

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
}

let toastId = 0;

export function App() {
  const [route, setRoute] = useState<"home" | "editor">("home");
  const [media, setMedia] = useState<MediaInfo_Serialize | null>(null);
  const [playback, setPlayback] = useState<PlaybackState>(EMPTY);
  const [backend, setBackend] = useState<ViewerBackend>("stream");
  const [platform, setPlatform] = useState("");
  const [diagnostics, setDiagnostics] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [layout, setLayout] = useState<Layout>({ panelOpen: true, timelineOpen: true, chatOpen: true, viewerOpen: true });

  const showError = useCallback((error: AppError) => {
    const id = ++toastId;
    setToasts((t) => [...t.slice(-3), { id, error }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 6000);
  }, []);

  const openPath = useCallback(
    async (path: string) => {
      const r = await commands.openMedia(path);
      if (r.status === "error") return showError(r.error);
      setMedia(r.data);
      setRoute("editor");
    },
    [showError],
  );

  const importMedia = useCallback(async () => {
    const r = await commands.openMediaDialog();
    if (r.status === "error") return showError(r.error);
    if (r.data) {
      setMedia(r.data);
      setRoute("editor");
    }
  }, [showError]);

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

  // Bridge wiring: app info, routed paths, playback and error events.
  useEffect(() => {
    const unlisten = [
      events.playbackChanged.listen((e) => setPlayback(e.payload)),
      events.appErrorRaised.listen((e) => showError(e.payload)),
      events.viewerBackendChanged.listen((e) => setBackend(e.payload)),
      events.openPathsRequested.listen((e) => {
        if (e.payload.paths.length > 0) void openPath(e.payload.paths[0]);
      }),
    ];
    void commands.appInfo().then((r) => {
      if (r.status === "ok") {
        setBackend(r.data.viewerBackend);
        setPlatform(r.data.os.toLowerCase().startsWith("mac") ? "macos" : r.data.os.toLowerCase());
      }
    });
    void commands.takePendingOpenPaths().then((r) => {
      if (r.status === "ok" && r.data.length > 0) void openPath(r.data[0]);
    });
    return () => unlisten.forEach((p) => void p.then((f) => f()));
  }, [openPath, showError]);

  // Native viewer under a transparent web view needs a transparent page behind the viewer box.
  useEffect(() => {
    document.documentElement.dataset.viewer = backend === "stream" ? "stream" : "native";
    document.documentElement.dataset.platform = platform;
  }, [backend, platform]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = keyToAction(e);
      if (!action) return;
      if (action.type === "toggleDiagnostics") {
        e.preventDefault();
        setDiagnostics((d) => !d);
      } else if (route === "editor") {
        e.preventDefault();
        transport(action.command);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [route, transport]);

  return (
    <div className="app">
      {route === "home" ? (
        <Home onOpenFile={() => void importMedia()} onBlank={() => setRoute("editor")} />
      ) : (
        <Editor
          media={media}
          playback={playback}
          backend={backend}
          diagnosticsOpen={diagnostics}
          layout={layout}
          setLayout={setLayout}
          onTransport={transport}
          onImport={() => void importMedia()}
          onError={showError}
          onHome={() => setRoute("home")}
        />
      )}
      <div className="toasts" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className="toast" data-testid="toast" data-code={t.error.code}>
            {t.error.message}
            <button className="icon-btn" aria-label="Dismiss" onClick={() => setToasts((x) => x.filter((y) => y.id !== t.id))}>
              ×
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
