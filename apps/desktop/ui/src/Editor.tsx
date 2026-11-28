import {
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import type {
  AppError,
  ItemView_Serialize,
  MediaInfo_Serialize,
  MediaView_Serialize,
  PlaybackState,
  ProjectView_Serialize,
  TransportCommand,
  ViewerBackend,
} from "./bindings";
import { commands } from "./bridge";
import { ChatPanel } from "./ChatPanel";
import { ErrorBoundary } from "./ErrorBoundary";
import { Icon, type IconName } from "./icons";
import { Dropdown } from "./Menu";
import { fire, runCommand } from "./notify";
import { formatShort, formatTimecode, parseTimecode } from "./timecode";
import { Viewer } from "./Viewer";
import { Meters } from "./Meters";
import { type EditResult, firstTrack, itemDetail, snapToFrame } from "./edit";
import { Inspector } from "./Inspector";
import { EDIT_ACTIONS, MOD, type Tool, keyToAction } from "./keys";
import { CaptionsPanel } from "./CaptionsPanel";
import { TranscriptPanel } from "./TranscriptPanel";
import {
  AudioPanel,
  EffectsPanel,
  LayoutPanel,
  type PanelProps,
  TextPanel,
  ExportsPanel,
  TransitionsPanel,
} from "./Panels";
import { RangeMediaDialog, openExport } from "./ExportDialog";
import { type ClipDrag, MediaPanel } from "./media/MediaPanel";
import type { MenuItem } from "./Menu";
import { MotionPanel } from "./Motion";
import { GeneratePanel } from "./Generate";
import { HistoryPanel } from "./HistoryPanel";
import { type HelpActions, HelpMenu } from "./onboarding/HelpMenu";
import { Shortcuts } from "./Shortcuts";
import { type SelectMode, Timeline } from "./Timeline";

export const RAIL: { id: string; label: string; icon: IconName }[] = [
  { id: "media", label: "Media", icon: "media" },
  { id: "layout", label: "Layout", icon: "layout" },
  { id: "audio", label: "Audio", icon: "audio" },
  { id: "captions", label: "Captions", icon: "captions" },
  { id: "transcript", label: "Transcript", icon: "scissors" },
  { id: "text", label: "Text", icon: "text" },
  { id: "motion", label: "Motion", icon: "templates" },
  { id: "transitions", label: "Transitions", icon: "transitions" },
  { id: "effects", label: "Effects", icon: "effects" },
  { id: "generate", label: "Generate", icon: "spark" },
  { id: "exports", label: "Exports", icon: "upload" },
];

export interface Layout {
  panelOpen: boolean;
  timelineOpen: boolean;
  chatOpen: boolean;
  viewerOpen: boolean;
}

/** Timeline time of the playhead: the previewed clip's start plus how far into its source the viewer is. */
export function playheadSeconds(
  item: ItemView_Serialize | undefined,
  playback: PlaybackState,
  previewing: boolean,
) {
  if (!item || !previewing) return 0;
  const fps = playback.frameRate ?? 30;
  const t = (item.start ?? 0) + playback.frame / fps - (item.trimStart ?? 0);
  return Math.min(Math.max(t, item.start ?? 0), item.end ?? 0);
}

export function Editor({
  project,
  selection,
  onSelect,
  onSelectMany,
  onEdit,
  media,
  playback,
  backend,
  diagnosticsOpen,
  layout,
  setLayout,
  onTransport,
  onImport,
  onImportTimeline,
  onError,
  onHome,
  onSeek,
  help: helpActions,
  timelineMode = false,
  barExtras,
}: {
  media: MediaInfo_Serialize | null;
  playback: PlaybackState;
  backend: ViewerBackend;
  diagnosticsOpen: boolean;
  layout: Layout;
  setLayout: (l: Layout) => void;
  onTransport: (c: TransportCommand) => void;
  onImport: () => void;
  /** Adds an FCPXML / XML / EDL / OTIO timeline to the project. */
  onImportTimeline?: () => void;
  onError: (e: AppError) => void;
  onHome: () => void;
  /** What the Help entry reopens (the welcome, the checklist, the sample). */
  help?: HelpActions;
  /** Shows the composited timeline at a time in seconds (the assistant's `seek_viewer`). */
  onSeek?: (seconds: number) => void;
  project: ProjectView_Serialize | null;
  /** Selected item ids, primary last. */
  selection: string[];
  onSelect: (
    item: ItemView_Serialize | null,
    offsetSeconds?: number,
    mode?: SelectMode,
  ) => void;
  onSelectMany: (ids: string[]) => void;
  onEdit: (p: EditResult) => void;
  /** The viewer plays the composited timeline: the playhead is the engine position. */
  timelineMode?: boolean;
  /** Extra controls at the end of the project bar. */
  barExtras?: ReactNode;
}) {
  const [panel, setPanel] = useState("media");
  const [zoom, setZoom] = useState<number | null>(null);
  const [fitPps, setFitPps] = useState(50);
  const [snap, setSnap] = useState(true);
  const [tool, setTool] = useState<Tool>("select");
  const [range, setRange] = useState<{ in: number | null; out: number | null }>({ in: null, out: null });
  const [help, setHelp] = useState(false);
  /** "Save range as media" or "Render in place" in progress. */
  const [rangeMedia, setRangeMedia] = useState<{ start: number; end: number; replace: boolean } | null>(null);
  const rangeMenu = (start: number, end: number): MenuItem[] => [
    { id: "export-range", label: "Export range…", onSelect: () => openExport([start, end]) },
    { id: "save-range", label: "Save range as media", onSelect: () => setRangeMedia({ start, end, replace: false }) },
    { id: "render-in-place", label: "Render in place…", onSelect: () => setRangeMedia({ start, end, replace: true }) },
  ];
  const [historyOpen, setHistoryOpen] = useState(false);
  /** Items an assistant turn changed, highlighted on the timeline while its "Show" is on (in the project it was
   * shown for: opening another project drops it). */
  const [shown, setReview] = useState<{
    projectId: string | null | undefined;
    turnId: string;
    ids: string[];
  } | null>(null);
  const review = shown && shown.projectId === project?.id ? shown : null;
  const [closedFor, setClosedFor] = useState<string | null>(null);
  const [scrubTime, setScrubTime] = useState(0);
  const [clipboard, setClipboard] = useState<Record<string, unknown>[]>([]);
  const toggle = (key: keyof Layout) =>
    setLayout({ ...layout, [key]: !layout[key] });
  const selected = selection[selection.length - 1] ?? null;
  const item = project?.items.find((i) => i.id === selected);
  const selectedItems = (project?.items ?? []).filter((i) =>
    selection.includes(i.id),
  );
  const itemMedia = project?.media.find((m) => m.id === item?.mediaId);
  const previewing = !!(
    item &&
    itemMedia &&
    media &&
    media.path === itemMedia.path
  );
  const playhead = timelineMode
    ? playback.frame / (playback.frameRate || 30)
    : previewing
      ? playheadSeconds(item, playback, previewing)
      : item && !item.mediaId
        ? (item.start ?? 0)
        : scrubTime;
  const inspecting = !!(
    item &&
    project &&
    closedFor !== selected &&
    layout.panelOpen
  );
  // Other parts of the app (the background-tasks popover, the export dialog) ask for a panel by id.
  useEffect(() => {
    const show = (e: Event) => {
      const id = (e as CustomEvent<string>).detail;
      if (!RAIL.some((r) => r.id === id)) return;
      // As the rail does: the panel takes the place of the selected clip's properties.
      setClosedFor(selected);
      setPanel(id);
      setLayout({ ...layout, panelOpen: true });
    };
    window.addEventListener("ove:open-panel", show);
    return () => window.removeEventListener("ove:open-panel", show);
  }, [layout, setLayout, selected]);

  const copy = useCallback(async (ids: string[]) => {
    const all = await Promise.all(ids.map((id) => itemDetail(id)));
    setClipboard(all.filter((x): x is Record<string, unknown> => x !== null));
  }, []);
  const paste = useCallback(
    (at: number) => {
      if (clipboard.length)
        onEdit(commands.pasteItems(JSON.stringify(clipboard), snapToFrame(at)));
    },
    [clipboard, onEdit],
  );
  const zoomBy = useCallback(
    (f: number) =>
      setZoom((z) => Math.min(400, Math.max(2, (z ?? fitPps) * f))),
    [fitPps],
  );

  /** Runs an edit verb through the shared planners (one revision; a refusal shows as an error). */
  const plan = useCallback((request: Record<string, unknown>) => onEdit(commands.editPlan(JSON.stringify(request))), [onEdit]);

  /** Inserts or overwrites the clip shown in the viewer (a project file previewed as the source) at the playhead. */
  const placeSource = useCallback(
    (op: "insert" | "overwrite") => {
      const src = project?.media.find((m) => media && m.path === media.path);
      if (!project || !src || timelineMode) {
        onError({ code: "invalid_argument", message: "Show a clip from the project in the viewer first (its source), then insert or overwrite it." });
        return;
      }
      const audio = src.kind === "audio";
      const track = audio ? firstTrack(project, "audio", "t_audio") : firstTrack(project, "video", "t_main");
      if (!track) return;
      const len = src.durationSeconds ?? 5;
      plan({
        op,
        track,
        at: snapToFrame(playhead),
        items: [{ type: audio ? "audio" : "video", mediaItemId: src.id, name: src.name, startTime: 0, endTime: len }],
      });
    },
    [project, media, timelineMode, playhead, plan, onError],
  );

  /** A Moment, Spoken result, subclip or media card dropped on a track: a clip playing that source range, placed over
   * whatever is there (one revision). */
  const dropClip = useCallback(
    (track: string, at: number, payload: string) => {
      let d: ClipDrag;
      try {
        d = JSON.parse(payload) as ClipDrag;
      } catch {
        return;
      }
      const type = d.kind === "audio" ? "audio" : d.kind === "image" ? "image" : "video";
      const len = type === "image" ? 5 : Math.max(0.04, d.end - d.start);
      const item: Record<string, unknown> = { type, mediaItemId: d.mediaId, name: d.name, startTime: 0, endTime: len };
      if (type !== "image" && d.start > 0) item.trimStart = d.start;
      plan({ op: "overwrite", track, at, items: [item] });
    },
    [plan],
  );

  // Editing shortcuts (transport, undo/redo and diagnostics are handled by the app).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const a = keyToAction(e);
      if (!a || !EDIT_ACTIONS.has(a.type)) return;
      e.preventDefault();
      const at = snapToFrame(playhead);
      switch (a.type) {
        case "tool":
          setTool(a.tool);
          break;
        case "markIn":
          setRange((r) => ({ in: at, out: r.out !== null && r.out > at ? r.out : null }));
          break;
        case "markOut":
          setRange((r) => ({ in: r.in !== null && r.in < at ? r.in : null, out: at }));
          break;
        case "markClip":
          if (item) setRange({ in: item.start ?? 0, out: item.end ?? 0 });
          break;
        case "addMarker":
          onEdit(commands.markerAdd(at, null));
          break;
        case "rippleTrimStart":
        case "rippleTrimEnd":
          if (item && at > (item.start ?? 0) && at < (item.end ?? 0))
            plan({ op: "rippleTrim", item: item.id, edge: a.type === "rippleTrimStart" ? "start" : "end", to: at });
          break;
        case "rippleDelete":
          if (range.in !== null && range.out !== null && range.out > range.in) {
            plan({ op: "extract", tracks: [], range: { start: range.in, end: range.out } });
            setRange({ in: null, out: null });
          } else if (selection.length) {
            plan({ op: "rippleDelete", items: selection });
            onSelectMany([]);
          }
          break;
        case "insert":
        case "overwrite":
          placeSource(a.type);
          break;
        case "delete":
          if (selection.length) {
            onEdit(commands.removeItems(selection));
            onSelectMany([]);
          }
          break;
        case "split":
          if (
            item &&
            playhead > (item.start ?? 0) &&
            playhead < (item.end ?? 0)
          )
            onEdit(commands.splitItem(item.id, playhead));
          break;
        case "duplicate":
          if (selection.length) onEdit(commands.duplicateItems(selection));
          break;
        case "copy":
          if (selection.length) fire(copy(selection), onError);
          break;
        case "paste":
          paste(playhead);
          break;
        case "selectAll": {
          const track = item?.trackId;
          onSelectMany(
            (project?.items ?? [])
              .filter((i) => !track || i.trackId === track)
              .map((i) => i.id),
          );
          break;
        }
        case "zoomIn":
          zoomBy(1.5);
          break;
        case "zoomOut":
          zoomBy(1 / 1.5);
          break;
        case "zoomFit":
          setZoom(null);
          break;
        case "help":
          setHelp((h) => !h);
          break;
        case "escape":
          if (help) setHelp(false);
          else onSelectMany([]);
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    selection,
    item,
    playhead,
    project,
    onEdit,
    onSelectMany,
    onError,
    copy,
    paste,
    zoomBy,
    help,
    plan,
    placeSource,
    range,
  ]);

  const panelProps: PanelProps | null = project
    ? {
        project,
        playhead,
        selection: selectedItems,
        onEdit,
        onSelect: (i) => onSelect(i),
        onError,
        onImport,
        onSeek,
        // Selected from a panel's hint: the panel stays, as when a rail button is pressed with a clip selected.
        onPick: (i) => {
          setClosedFor(i.id);
          onSelect(i);
        },
      }
    : null;
  const projectBar = project && (
    <div className="project-bar" data-testid="project-bar">
      <ProjectName
        name={project.name}
        onRename={(n) => onEdit(commands.renameProject(n))}
      />
      <Dropdown
        testId="aspect"
        label={project.aspectRatio}
        items={["16:9", "9:16", "1:1", "4:5", "4:3", "3:4"].map((a) => ({
          id: a,
          label: a,
          onSelect: () => onEdit(commands.setAspectRatio(a)),
        }))}
      />
      <span
        className="muted small"
        title="Saved continuously; every change is a revision"
      >
        Saved · rev {String(project.rev ?? 0).slice(-6)}
      </span>
      {barExtras}
    </div>
  );

  return (
    <div
      className={`editor${layout.chatOpen ? "" : " no-chat"}`}
      data-testid="editor"
    >
      <div className="editor-main">
        <div
          className={`editor-top${layout.viewerOpen ? "" : " viewer-collapsed"}`}
        >
          <nav className="rail" aria-label="Editor panels">
            <button
              className="rail-home"
              title="Home"
              onClick={onHome}
              data-testid="go-home"
            >
              <Icon name="home" />
            </button>
            {RAIL.map((r) => (
              <button
                key={r.id}
                className={`rail-item${panel === r.id && layout.panelOpen ? " active" : ""}`}
                title={r.label}
                aria-label={r.label}
                data-testid={`rail-${r.id}`}
                onClick={() => {
                  if (inspecting) {
                    setClosedFor(selected);
                    setPanel(r.id);
                  } else if (panel === r.id)
                    setLayout({ ...layout, panelOpen: !layout.panelOpen });
                  else {
                    setPanel(r.id);
                    setLayout({ ...layout, panelOpen: true });
                  }
                }}
              >
                <Icon name={r.icon} size={19} />
              </button>
            ))}
          </nav>
          {inspecting && item && project ? (
            <ErrorBoundary
              region="The inspector"
              className="side-panel"
              resetKeys={[item.id]}
            >
              <Inspector
                key={item.id}
                project={project}
                item={item}
                onEdit={onEdit}
                onClose={() => setClosedFor(selected)}
                onCollapse={() => toggle("panelOpen")}
                playhead={playhead}
              />
            </ErrorBoundary>
          ) : (
            layout.panelOpen && (
              <ErrorBoundary
                region={`The ${RAIL.find((r) => r.id === panel)?.label ?? ""} panel`}
                className="side-panel"
                resetKeys={[panel]}
              >
                <SidePanel
                  id={panel}
                  project={project}
                  panelProps={panelProps}
                  onImport={onImport}
                  onImportTimeline={onImportTimeline}
                  onCollapse={() => toggle("panelOpen")}
                  onPick={(m) => {
                    const it = project?.items.find((i) => i.mediaId === m.id);
                    if (it) onSelect(it);
                  }}
                />
              </ErrorBoundary>
            )
          )}
          <ErrorBoundary region="The viewer" className="viewer-area">
            <Viewer
              media={media}
              frame={playback.frame}
              backend={backend}
              visible={layout.viewerOpen}
              diagnosticsOpen={diagnosticsOpen}
              onError={onError}
              toolbarLeft={projectBar}
            />
          </ErrorBoundary>
          {layout.viewerOpen && <Meters />}
        </div>
        <TransportBar
          playback={playback}
          onTransport={onTransport}
          layout={layout}
          toggle={toggle}
          project={project}
          canSplit={
            !!item && playhead > (item.start ?? 0) && playhead < (item.end ?? 0)
          }
          onEdit={onEdit}
          onSplit={() => item && onEdit(commands.splitItem(item.id, playhead))}
          tool={tool}
          setTool={setTool}
          zoom={zoom}
          fitPps={fitPps}
          setZoom={setZoom}
          snap={snap}
          setSnap={setSnap}
          onHelp={() => setHelp(true)}
          help={helpActions}
          onHistory={() => setHistoryOpen(true)}
        />
        {layout.timelineOpen ? (
          <ErrorBoundary region="The timeline" className="timeline">
            <Timeline
              project={project}
              onDropClip={dropClip}
              selection={selection}
              playhead={playhead}
              zoom={zoom}
              snap={snap}
              onSelect={(i, offset, mode) => {
                // Clicking a clip brings its properties back after a tool panel was opened.
                setClosedFor(null);
                onSelect(i, offset, mode);
              }}
              onScrub={(t) => {
                setScrubTime(t);
                onSelectMany([]);
                if (timelineMode)
                  onTransport({
                    kind: "seekFrame",
                    frame: Math.round(t * (playback.frameRate || 30)),
                  });
              }}
              onEdit={onEdit}
              onCopy={(ids) => fire(copy(ids), onError)}
              onPaste={paste}
              canPaste={clipboard.length > 0}
              onFitWidth={setFitPps}
              highlight={review?.ids}
              tool={tool}
              range={range}
              rangeMenu={project ? rangeMenu : undefined}
            />
          </ErrorBoundary>
        ) : (
          <ScrubRow playback={playback} onTransport={onTransport} />
        )}
      </div>
      {layout.chatOpen && (
        <ErrorBoundary
          region="The assistant"
          className="chat"
          onClose={() => toggle("chatOpen")}
        >
          <ChatPanel
            onClose={() => toggle("chatOpen")}
            projectId={project?.id}
            onError={onError}
            onSeek={onSeek}
            shownTurn={review?.turnId ?? null}
            onShowTurn={(turnId, ids) =>
              setReview(
                turnId ? { projectId: project?.id, turnId, ids } : null,
              )
            }
          />
        </ErrorBoundary>
      )}
      {help && <Shortcuts onClose={() => setHelp(false)} />}
      {historyOpen && (
        <HistoryPanel
          rev={project?.rev}
          onClose={() => setHistoryOpen(false)}
          onEdit={onEdit}
          onError={onError}
        />
      )}
      {rangeMedia && (
        <RangeMediaDialog
          {...rangeMedia}
          onClose={() => setRangeMedia(null)}
          onError={onError}
        />
      )}
    </div>
  );
}

function ProjectName({
  name,
  onRename,
}: {
  name: string;
  onRename: (n: string) => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  if (editing !== null)
    return (
      <input
        className="project-name-input"
        autoFocus
        value={editing}
        aria-label="Project name"
        onChange={(e) => setEditing(e.target.value)}
        onBlur={() => setEditing(null)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setEditing(null);
          if (e.key === "Enter") {
            if (editing.trim() && editing !== name) onRename(editing.trim());
            setEditing(null);
          }
        }}
      />
    );
  return (
    <button
      className="project-name"
      title="Rename project"
      onClick={() => setEditing(name)}
      data-testid="project-name"
    >
      {name}
    </button>
  );
}


function SidePanel({
  id,
  project,
  panelProps,
  onImport,
  onImportTimeline,
  onCollapse,
  onPick,
}: {
  id: string;
  project: ProjectView_Serialize | null;
  panelProps: PanelProps | null;
  onImport: () => void;
  onImportTimeline?: () => void;
  onCollapse: () => void;
  onPick: (m: MediaView_Serialize) => void;
}) {
  const label = RAIL.find((r) => r.id === id)?.label ?? "";
  return (
    <aside className="side-panel" data-testid="side-panel">
      <header className="panel-header">
        <h2>{label}</h2>
        <button
          className="icon-btn"
          title="Collapse panel"
          onClick={onCollapse}
          data-testid="collapse-panel"
        >
          <Icon name="panel" />
        </button>
      </header>
      {id === "media" ? (
        <MediaPanel
          project={project}
          panelProps={panelProps}
          onImport={onImport}
          onImportTimeline={onImportTimeline}
          onPick={onPick}
        />
      ) : panelProps ? (
        <div className="panel-body" data-testid={`panel-${id}`}>
          {id === "text" && <TextPanel {...panelProps} />}
          {id === "motion" && <MotionPanel {...panelProps} />}
          {id === "captions" && <CaptionsPanel {...panelProps} />}
          {id === "transcript" && <TranscriptPanel {...panelProps} />}
          {id === "transitions" && <TransitionsPanel {...panelProps} />}
          {id === "effects" && <EffectsPanel {...panelProps} />}
          {id === "audio" && <AudioPanel {...panelProps} />}
          {id === "layout" && <LayoutPanel {...panelProps} />}
          {id === "generate" && <GeneratePanel {...panelProps} />}
          {id === "exports" && <ExportsPanel onError={panelProps.onError} />}
        </div>
      ) : (
        <p className="muted panel-placeholder">
          Open a project to use {label.toLowerCase()} tools.
        </p>
      )}
    </aside>
  );
}

function TimecodeField({
  playback,
  onTransport,
}: {
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  const [invalid, setInvalid] = useState(false);
  if (editing !== null) {
    return (
      <input
        className={`timecode-input${invalid ? " invalid" : ""}`}
        autoFocus
        value={editing}
        data-testid="timecode-input"
        aria-label="Go to timecode"
        onChange={(e) => {
          setEditing(e.target.value);
          setInvalid(false);
        }}
        onBlur={() => setEditing(null)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setEditing(null);
          if (e.key === "Enter") {
            const frame = parseTimecode(
              editing,
              playback.frameRate,
              playback.totalFrames,
            );
            if (frame === null) setInvalid(true);
            else {
              onTransport({ kind: "seekFrame", frame });
              setEditing(null);
            }
          }
        }}
      />
    );
  }
  return (
    <button
      className="timecode"
      data-testid="timecode"
      title="Click to enter a timecode"
      onClick={() =>
        setEditing(formatTimecode(playback.frame, playback.frameRate))
      }
    >
      {formatTimecode(playback.frame, playback.frameRate)}
    </button>
  );
}

function TransportBar({
  playback,
  onTransport,
  layout,
  toggle,
  project,
  canSplit,
  onEdit,
  onSplit,
  zoom,
  fitPps,
  setZoom,
  snap,
  setSnap,
  onHelp,
  help,
  onHistory,
  tool,
  setTool,
}: {
  tool: Tool;
  setTool: (t: Tool) => void;
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
  layout: Layout;
  toggle: (k: keyof Layout) => void;
  project: ProjectView_Serialize | null;
  canSplit: boolean;
  onEdit: (p: EditResult) => void;
  onSplit: () => void;
  zoom: number | null;
  fitPps: number;
  setZoom: (z: number | null) => void;
  snap: boolean;
  setSnap: (v: boolean) => void;
  /** Shows the keyboard shortcuts. */
  onHelp: () => void;
  help?: HelpActions;
  onHistory: () => void;
}) {
  const pps = zoom ?? fitPps;
  const volume = playback.volume ?? 1;
  const setVolume = (v: number, muted: boolean) =>
    runCommand(commands.setVolume(v, muted));
  return (
    <div className="transport" data-testid="transport">
      <div className="transport-left">
        <TimecodeField playback={playback} onTransport={onTransport} />
        <span className="muted small">
          {formatShort(playback.frame, playback.frameRate)} /{" "}
          {formatShort(playback.totalFrames, playback.frameRate)}
        </span>
        <span className="sep" />
        <button
          className="icon-btn"
          title={
            project?.undoSummary
              ? `Undo ${project.undoSummary} (${MOD} Z)`
              : `Undo (${MOD} Z)`
          }
          disabled={!project?.canUndo}
          data-testid="undo"
          onClick={() => onEdit(commands.undo())}
        >
          <Icon name="undo" size={16} />
        </button>
        <button
          className="icon-btn"
          title={`Redo (⇧${MOD} Z)`}
          disabled={!project?.canRedo}
          data-testid="redo"
          onClick={() => onEdit(commands.redo())}
        >
          <Icon name="redo" size={16} />
        </button>
        <button
          className="icon-btn"
          title="History: every change, and who made it"
          aria-label="History"
          disabled={!project}
          data-testid="history"
          onClick={onHistory}
        >
          <Icon name="history" size={16} />
        </button>
        <button
          className="icon-btn"
          title="Split the selected clip at the playhead (S)"
          disabled={!canSplit}
          data-testid="split"
          onClick={onSplit}
        >
          <Icon name="scissors" size={16} />
        </button>
        <span className="tool-group" role="group" aria-label="Timeline tool">
          {(
            [
              ["select", "Select tool: move and trim (V)", "V"],
              ["blade", "Blade tool: click a clip to cut it (B)", "B"],
              ["trim", "Trim tool: drag to slip, Alt-drag to slide, drag an edit point to roll (T)", "T"],
            ] as const
          ).map(([id, title, label]) => (
            <button
              key={id}
              className={`icon-btn tool-btn${tool === id ? " on" : ""}`}
              title={title}
              aria-pressed={tool === id}
              data-testid={`tool-${id}`}
              onClick={() => setTool(id)}
            >
              {id === "blade" ? <Icon name="scissors" size={14} /> : label}
            </button>
          ))}
        </span>
        <button
          className={`icon-btn${snap ? " on" : ""}`}
          title={snap ? "Snapping on" : "Snapping off"}
          aria-pressed={snap}
          data-testid="snap-toggle"
          onClick={() => setSnap(!snap)}
        >
          <Icon name="magnet" size={16} />
        </button>
      </div>
      <div className="transport-center">
        <button
          className="icon-btn"
          title="Go to start (Home)"
          aria-label="Go to start"
          onClick={() => onTransport({ kind: "goToStart" })}
        >
          <Icon name="toStart" size={16} />
        </button>
        <button
          className="icon-btn"
          title="Previous frame (←)"
          aria-label="Previous frame"
          onClick={() => onTransport({ kind: "step", frames: -1 })}
        >
          <Icon name="stepBack" size={16} />
        </button>
        <button
          className="icon-btn play"
          data-testid="play"
          title="Play/Pause (Space)"
          aria-label={playback.playing ? "Pause" : "Play"}
          onClick={() => onTransport({ kind: "togglePlay" })}
        >
          <Icon name={playback.playing ? "pause" : "play"} size={18} />
        </button>
        <button
          className="icon-btn"
          title="Next frame (→)"
          aria-label="Next frame"
          onClick={() => onTransport({ kind: "step", frames: 1 })}
        >
          <Icon name="stepForward" size={16} />
        </button>
        <button
          className="icon-btn"
          title="Go to end (End)"
          aria-label="Go to end"
          onClick={() => onTransport({ kind: "goToEnd" })}
        >
          <Icon name="toEnd" size={16} />
        </button>
        <button
          className={`icon-btn${playback.looping ? " on" : ""}`}
          title="Loop"
          aria-pressed={playback.looping}
          data-testid="loop"
          onClick={() =>
            onTransport({ kind: "setLoop", enabled: !playback.looping })
          }
        >
          <Icon name="loop" size={16} />
        </button>
        {playback.speed !== null && Math.abs(playback.speed) > 1 && (
          <span className="speed" data-testid="speed">
            {playback.speed > 0 ? "" : "−"}
            {Math.abs(playback.speed)}×
          </span>
        )}
      </div>
      <div className="transport-right">
        <div className="zoom-group" data-testid="timeline-zoom">
          <button
            className="icon-btn"
            title="Zoom out (−)"
            onClick={() => setZoom(Math.max(2, pps / 1.5))}
          >
            <Icon name="zoomOut" size={15} />
          </button>
          <input
            className="zoom-slider"
            type="range"
            min={0}
            max={1}
            step={0.01}
            aria-label="Timeline zoom"
            value={Math.log(pps / 2) / Math.log(200)}
            onChange={(e) => setZoom(2 * Math.pow(200, Number(e.target.value)))}
          />
          <button
            className="icon-btn"
            title="Zoom in (+)"
            onClick={() => setZoom(Math.min(400, pps * 1.5))}
          >
            <Icon name="zoomIn" size={15} />
          </button>
          <button
            className={`chip-sm${zoom === null ? " on" : ""}`}
            title={`Fit the timeline (${MOD} 0)`}
            data-testid="zoom-fit"
            onClick={() => setZoom(null)}
          >
            Fit
          </button>
        </div>
        <span className="sep" />
        <button
          className="icon-btn"
          title={playback.muted ? "Unmute" : "Mute"}
          data-testid="mute"
          onClick={() => setVolume(volume, !playback.muted)}
        >
          <Icon name={playback.muted ? "mute" : "volume"} size={16} />
        </button>
        <input
          className="volume"
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={playback.muted ? 0 : volume}
          aria-label="Volume"
          onChange={(e) => setVolume(Number(e.target.value), false)}
        />
        <span className="sep" />
        <button
          className={`icon-btn${layout.viewerOpen ? "" : " on"}`}
          title="Expand timeline (hide viewer)"
          data-testid="toggle-viewer"
          onClick={() => toggle("viewerOpen")}
        >
          <Icon name="expand" size={16} />
        </button>
        <button
          className={`icon-btn${layout.timelineOpen ? " on" : ""}`}
          title="Show timeline"
          data-testid="toggle-timeline"
          onClick={() => toggle("timelineOpen")}
        >
          <Icon name="timeline" size={16} />
        </button>
        <button
          className={`icon-btn${layout.chatOpen ? " on" : ""}`}
          title="Show assistant"
          data-testid="toggle-chat"
          onClick={() => toggle("chatOpen")}
        >
          <Icon name="chat" size={16} />
        </button>
        <HelpMenu variant="icon" actions={help} onShortcuts={onHelp} />
      </div>
    </div>
  );
}

function useScrub(
  playback: PlaybackState,
  onTransport: (c: TransportCommand) => void,
) {
  const el = useRef<HTMLDivElement>(null);
  const seekAt = (clientX: number, offset = 0) => {
    const r = el.current?.getBoundingClientRect();
    if (!r || playback.totalFrames <= 0) return;
    const x = Math.min(
      Math.max(0, clientX - r.left - offset),
      r.width - offset,
    );
    const frame = Math.round(
      (x / Math.max(1, r.width - offset)) * (playback.totalFrames - 1),
    );
    onTransport({ kind: "seekFrame", frame });
  };
  const onPointerDown = (e: React.PointerEvent, offset = 0) => {
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
    seekAt(e.clientX, offset);
    const move = (ev: PointerEvent) => seekAt(ev.clientX, offset);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  return { el, onPointerDown };
}

function ScrubRow({
  playback,
  onTransport,
}: {
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
}) {
  const { el, onPointerDown } = useScrub(playback, onTransport);
  const pct =
    playback.totalFrames > 1
      ? (playback.frame / (playback.totalFrames - 1)) * 100
      : 0;
  return (
    <div className="scrub-row" data-testid="scrub-row">
      <span className="small">
        {formatTimecode(playback.frame, playback.frameRate)}
      </span>
      <div
        className="scrub"
        ref={el}
        onPointerDown={(e) => onPointerDown(e)}
        data-testid="scrub"
      >
        <div className="scrub-fill" style={{ width: `${pct}%` }} />
        <div className="scrub-knob" style={{ left: `${pct}%` }} />
      </div>
      <span className="small">
        {formatTimecode(
          Math.max(0, playback.totalFrames - 1),
          playback.frameRate,
        )}
      </span>
    </div>
  );
}
