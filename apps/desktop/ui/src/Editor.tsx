import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
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
import { Icon, type IconName } from "./icons";
import { Dropdown } from "./Menu";
import { formatShort, formatTimecode, parseTimecode } from "./timecode";
import { Viewer } from "./Viewer";
import { type EditResult, itemDetail, snapToFrame } from "./edit";
import { Inspector } from "./Inspector";
import { EDIT_ACTIONS, keyToAction } from "./keys";
import { CaptionsPanel } from "./CaptionsPanel";
import { AudioPanel, EffectsPanel, LayoutPanel, type PanelProps, TextPanel, TransitionsPanel } from "./Panels";
import { Shortcuts } from "./Shortcuts";
import { type SelectMode, Timeline } from "./Timeline";

export const RAIL: { id: string; label: string; icon: IconName }[] = [
  { id: "media", label: "Media", icon: "media" },
  { id: "layout", label: "Layout", icon: "layout" },
  { id: "audio", label: "Audio", icon: "audio" },
  { id: "captions", label: "Captions", icon: "captions" },
  { id: "text", label: "Text", icon: "text" },
  { id: "transitions", label: "Transitions", icon: "transitions" },
  { id: "effects", label: "Effects", icon: "effects" },
];

export interface Layout {
  panelOpen: boolean;
  timelineOpen: boolean;
  chatOpen: boolean;
  viewerOpen: boolean;
}

/** Timeline time of the playhead: the previewed clip's start plus how far into its source the viewer is. */
export function playheadSeconds(item: ItemView_Serialize | undefined, playback: PlaybackState, previewing: boolean) {
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
  onError,
  onHome,
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
  onError: (e: AppError) => void;
  onHome: () => void;
  project: ProjectView_Serialize | null;
  /** Selected item ids, primary last. */
  selection: string[];
  onSelect: (item: ItemView_Serialize | null, offsetSeconds?: number, mode?: SelectMode) => void;
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
  const [help, setHelp] = useState(false);
  const [closedFor, setClosedFor] = useState<string | null>(null);
  const [scrubTime, setScrubTime] = useState(0);
  const [clipboard, setClipboard] = useState<Record<string, unknown>[]>([]);
  const toggle = (key: keyof Layout) => setLayout({ ...layout, [key]: !layout[key] });
  const selected = selection[selection.length - 1] ?? null;
  const item = project?.items.find((i) => i.id === selected);
  const selectedItems = (project?.items ?? []).filter((i) => selection.includes(i.id));
  const itemMedia = project?.media.find((m) => m.id === item?.mediaId);
  const previewing = !!(item && itemMedia && media && media.path === itemMedia.path);
  const playhead = timelineMode
    ? playback.frame / (playback.frameRate || 30)
    : previewing
      ? playheadSeconds(item, playback, previewing)
      : item && !item.mediaId
        ? (item.start ?? 0)
        : scrubTime;
  const inspecting = !!(item && project && closedFor !== selected && layout.panelOpen);

  const copy = useCallback(async (ids: string[]) => {
    const all = await Promise.all(ids.map((id) => itemDetail(id)));
    setClipboard(all.filter((x): x is Record<string, unknown> => x !== null));
  }, []);
  const paste = useCallback(
    (at: number) => {
      if (clipboard.length) onEdit(commands.pasteItems(JSON.stringify(clipboard), snapToFrame(at)));
    },
    [clipboard, onEdit],
  );
  const zoomBy = useCallback((f: number) => setZoom((z) => Math.min(400, Math.max(2, (z ?? fitPps) * f))), [fitPps]);

  // Editing shortcuts (transport, undo/redo and diagnostics are handled by the app).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const a = keyToAction(e);
      if (!a || !EDIT_ACTIONS.has(a.type)) return;
      e.preventDefault();
      switch (a.type) {
        case "delete":
          if (selection.length) {
            onEdit(commands.removeItems(selection));
            onSelectMany([]);
          }
          break;
        case "split":
          if (item && playhead > (item.start ?? 0) && playhead < (item.end ?? 0)) onEdit(commands.splitItem(item.id, playhead));
          break;
        case "duplicate":
          if (selection.length) onEdit(commands.duplicateItems(selection));
          break;
        case "copy":
          if (selection.length) void copy(selection);
          break;
        case "paste":
          paste(playhead);
          break;
        case "selectAll": {
          const track = item?.trackId;
          onSelectMany((project?.items ?? []).filter((i) => !track || i.trackId === track).map((i) => i.id));
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
  }, [selection, item, playhead, project, onEdit, onSelectMany, copy, paste, zoomBy, help]);

  const panelProps: PanelProps | null = project
    ? { project, playhead, selection: selectedItems, onEdit, onSelect: (i) => onSelect(i) }
    : null;
  const projectBar = project && (
    <div className="project-bar" data-testid="project-bar">
      <ProjectName name={project.name} onRename={(n) => onEdit(commands.renameProject(n))} />
      <Dropdown
        testId="aspect"
        label={project.aspectRatio}
        items={["16:9", "9:16", "1:1", "4:5", "4:3", "3:4"].map((a) => ({
          id: a,
          label: a,
          onSelect: () => onEdit(commands.setAspectRatio(a)),
        }))}
      />
      <span className="muted small" title="Saved continuously; every change is a revision">
        Saved · rev {String(project.rev ?? 0).slice(-6)}
      </span>
      {barExtras}
    </div>
  );

  return (
    <div className={`editor${layout.chatOpen ? "" : " no-chat"}`} data-testid="editor">
      <div className="editor-main">
        <div className={`editor-top${layout.viewerOpen ? "" : " viewer-collapsed"}`}>
          <nav className="rail" aria-label="Editor panels">
            <button className="rail-home" title="Home" onClick={onHome} data-testid="go-home">
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
                  } else if (panel === r.id) setLayout({ ...layout, panelOpen: !layout.panelOpen });
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
            <Inspector
              key={item.id}
              project={project}
              item={item}
              onEdit={onEdit}
              onClose={() => setClosedFor(selected)}
              onCollapse={() => toggle("panelOpen")}
            />
          ) : (
            layout.panelOpen && (
              <SidePanel
                id={panel}
                project={project}
                panelProps={panelProps}
                onImport={onImport}
                onCollapse={() => toggle("panelOpen")}
                onPick={(m) => {
                  const it = project?.items.find((i) => i.mediaId === m.id);
                  if (it) onSelect(it);
                }}
              />
            )
          )}
          <Viewer
            media={media}
            frame={playback.frame}
            backend={backend}
            visible={layout.viewerOpen}
            diagnosticsOpen={diagnosticsOpen}
            onError={onError}
            toolbarLeft={projectBar}
          />
        </div>
        <TransportBar
          playback={playback}
          onTransport={onTransport}
          layout={layout}
          toggle={toggle}
          project={project}
          canSplit={!!item && playhead > (item.start ?? 0) && playhead < (item.end ?? 0)}
          onEdit={onEdit}
          onSplit={() => item && onEdit(commands.splitItem(item.id, playhead))}
          zoom={zoom}
          fitPps={fitPps}
          setZoom={setZoom}
          snap={snap}
          setSnap={setSnap}
          onHelp={() => setHelp(true)}
        />
        {layout.timelineOpen ? (
          <Timeline
            project={project}
            selection={selection}
            playhead={playhead}
            zoom={zoom}
            snap={snap}
            onSelect={onSelect}
            onScrub={(t) => {
              setScrubTime(t);
              onSelectMany([]);
              if (timelineMode) onTransport({ kind: "seekFrame", frame: Math.round(t * (playback.frameRate || 30)) });
            }}
            onEdit={onEdit}
            onCopy={(ids) => void copy(ids)}
            onPaste={paste}
            canPaste={clipboard.length > 0}
            onFitWidth={setFitPps}
          />
        ) : (
          <ScrubRow playback={playback} onTransport={onTransport} />
        )}
      </div>
      {layout.chatOpen && <ChatPanel onClose={() => toggle("chatOpen")} projectId={project?.id} onError={onError} />}
      {help && <Shortcuts onClose={() => setHelp(false)} />}
    </div>
  );
}

function ProjectName({ name, onRename }: { name: string; onRename: (n: string) => void }) {
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
    <button className="project-name" title="Rename project" onClick={() => setEditing(name)} data-testid="project-name">
      {name}
    </button>
  );
}

function fmtSeconds(s: number | null | undefined) {
  const v = Math.max(0, Math.round(s ?? 0));
  return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, "0")}`;
}

function SidePanel({
  id,
  project,
  panelProps,
  onImport,
  onCollapse,
  onPick,
}: {
  id: string;
  project: ProjectView_Serialize | null;
  panelProps: PanelProps | null;
  onImport: () => void;
  onCollapse: () => void;
  onPick: (m: MediaView_Serialize) => void;
}) {
  const label = RAIL.find((r) => r.id === id)?.label ?? "";
  const [query, setQuery] = useState("");
  const media = (project?.media ?? []).filter((m) => m.name.toLowerCase().includes(query.toLowerCase()));
  const used = new Set(project?.items.map((i) => i.mediaId));
  return (
    <aside className="side-panel" data-testid="side-panel">
      <header className="panel-header">
        <h2>{label}</h2>
        <button className="icon-btn" title="Collapse panel" onClick={onCollapse} data-testid="collapse-panel">
          <Icon name="panel" />
        </button>
      </header>
      {id === "media" ? (
        <>
          <div className="panel-search">
            <label className="search">
              <Icon name="search" size={15} />
              <input placeholder="Search media…" value={query} onChange={(e) => setQuery(e.target.value)} />
            </label>
            <button className="btn" onClick={onImport} data-testid="import">
              <Icon name="plus" size={15} /> Import
            </button>
          </div>
          <div className="media-grid">
            {media.map((m) => (
              <button key={m.id} className="media-card" title={m.path} onClick={() => onPick(m)} data-testid="media-card">
                <div className="media-thumb">
                  {m.thumbnail ? <img src={m.thumbnail} alt="" /> : <Icon name={m.kind === "audio" ? "audio" : "media"} size={22} />}
                  <span className="badge">{fmtSeconds(m.durationSeconds)}</span>
                  {m.status !== "online" && <span className="badge warn">{m.status}</span>}
                  {used.has(m.id) && <span className="badge added">On timeline</span>}
                </div>
                <div className="media-name">
                  <Icon name={m.kind === "audio" ? "audio" : "play"} size={12} /> {m.name}
                </div>
              </button>
            ))}
            {media.length === 0 && (
              <p className="muted">{query ? "No media matches." : "No media yet. Import files or drop them on the window."}</p>
            )}
          </div>
        </>
      ) : panelProps ? (
        <div className="panel-body" data-testid={`panel-${id}`}>
          {id === "text" && <TextPanel {...panelProps} />}
          {id === "captions" && <CaptionsPanel {...panelProps} />}
          {id === "transitions" && <TransitionsPanel {...panelProps} />}
          {id === "effects" && <EffectsPanel {...panelProps} />}
          {id === "audio" && <AudioPanel {...panelProps} />}
          {id === "layout" && <LayoutPanel {...panelProps} />}
        </div>
      ) : (
        <p className="muted panel-placeholder">Open a project to use {label.toLowerCase()} tools.</p>
      )}
    </aside>
  );
}

function TimecodeField({ playback, onTransport }: { playback: PlaybackState; onTransport: (c: TransportCommand) => void }) {
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
            const frame = parseTimecode(editing, playback.frameRate, playback.totalFrames);
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
      onClick={() => setEditing(formatTimecode(playback.frame, playback.frameRate))}
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
}: {
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
  onHelp: () => void;
}) {
  const pps = zoom ?? fitPps;
  const volume = playback.volume ?? 1;
  const setVolume = (v: number, muted: boolean) => void commands.setVolume(v, muted);
  return (
    <div className="transport" data-testid="transport">
      <div className="transport-left">
        <TimecodeField playback={playback} onTransport={onTransport} />
        <span className="muted small">
          {formatShort(playback.frame, playback.frameRate)} / {formatShort(playback.totalFrames, playback.frameRate)}
        </span>
        <span className="sep" />
        <button
          className="icon-btn"
          title={project?.undoSummary ? `Undo ${project.undoSummary} (⌘Z)` : "Undo (⌘Z)"}
          disabled={!project?.canUndo}
          data-testid="undo"
          onClick={() => onEdit(commands.undo())}
        >
          <Icon name="undo" size={16} />
        </button>
        <button className="icon-btn" title="Redo (⇧⌘Z)" disabled={!project?.canRedo} data-testid="redo" onClick={() => onEdit(commands.redo())}>
          <Icon name="redo" size={16} />
        </button>
        <button className="icon-btn" title="Split the selected clip at the playhead (S)" disabled={!canSplit} data-testid="split" onClick={onSplit}>
          <Icon name="scissors" size={16} />
        </button>
        <button className={`icon-btn${snap ? " on" : ""}`} title={snap ? "Snapping on" : "Snapping off"} aria-pressed={snap} data-testid="snap-toggle" onClick={() => setSnap(!snap)}>
          <Icon name="magnet" size={16} />
        </button>
      </div>
      <div className="transport-center">
        <button className="icon-btn" title="Go to start (Home)" aria-label="Go to start" onClick={() => onTransport({ kind: "goToStart" })}>
          <Icon name="toStart" size={16} />
        </button>
        <button className="icon-btn" title="Previous frame (←)" aria-label="Previous frame" onClick={() => onTransport({ kind: "step", frames: -1 })}>
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
        <button className="icon-btn" title="Next frame (→)" aria-label="Next frame" onClick={() => onTransport({ kind: "step", frames: 1 })}>
          <Icon name="stepForward" size={16} />
        </button>
        <button className="icon-btn" title="Go to end (End)" aria-label="Go to end" onClick={() => onTransport({ kind: "goToEnd" })}>
          <Icon name="toEnd" size={16} />
        </button>
        <button
          className={`icon-btn${playback.looping ? " on" : ""}`}
          title="Loop"
          aria-pressed={playback.looping}
          data-testid="loop"
          onClick={() => onTransport({ kind: "setLoop", enabled: !playback.looping })}
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
          <button className="icon-btn" title="Zoom out (−)" onClick={() => setZoom(Math.max(2, pps / 1.5))}>
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
          <button className="icon-btn" title="Zoom in (+)" onClick={() => setZoom(Math.min(400, pps * 1.5))}>
            <Icon name="zoomIn" size={15} />
          </button>
          <button className={`chip-sm${zoom === null ? " on" : ""}`} title="Fit the timeline (⌘0)" data-testid="zoom-fit" onClick={() => setZoom(null)}>
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
        <button className={`icon-btn${layout.viewerOpen ? "" : " on"}`} title="Expand timeline (hide viewer)" data-testid="toggle-viewer" onClick={() => toggle("viewerOpen")}>
          <Icon name="expand" size={16} />
        </button>
        <button className={`icon-btn${layout.timelineOpen ? " on" : ""}`} title="Show timeline" data-testid="toggle-timeline" onClick={() => toggle("timelineOpen")}>
          <Icon name="timeline" size={16} />
        </button>
        <button className={`icon-btn${layout.chatOpen ? " on" : ""}`} title="Show assistant" data-testid="toggle-chat" onClick={() => toggle("chatOpen")}>
          <Icon name="chat" size={16} />
        </button>
        <button className="icon-btn" title="Keyboard shortcuts (?)" data-testid="shortcuts" onClick={onHelp}>
          <Icon name="keyboard" size={16} />
        </button>
      </div>
    </div>
  );
}

function useScrub(playback: PlaybackState, onTransport: (c: TransportCommand) => void) {
  const el = useRef<HTMLDivElement>(null);
  const seekAt = (clientX: number, offset = 0) => {
    const r = el.current?.getBoundingClientRect();
    if (!r || playback.totalFrames <= 0) return;
    const x = Math.min(Math.max(0, clientX - r.left - offset), r.width - offset);
    const frame = Math.round((x / Math.max(1, r.width - offset)) * (playback.totalFrames - 1));
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

function ScrubRow({ playback, onTransport }: { playback: PlaybackState; onTransport: (c: TransportCommand) => void }) {
  const { el, onPointerDown } = useScrub(playback, onTransport);
  const pct = playback.totalFrames > 1 ? (playback.frame / (playback.totalFrames - 1)) * 100 : 0;
  return (
    <div className="scrub-row" data-testid="scrub-row">
      <span className="small">{formatTimecode(playback.frame, playback.frameRate)}</span>
      <div className="scrub" ref={el} onPointerDown={(e) => onPointerDown(e)} data-testid="scrub">
        <div className="scrub-fill" style={{ width: `${pct}%` }} />
        <div className="scrub-knob" style={{ left: `${pct}%` }} />
      </div>
      <span className="small">{formatTimecode(Math.max(0, playback.totalFrames - 1), playback.frameRate)}</span>
    </div>
  );
}

