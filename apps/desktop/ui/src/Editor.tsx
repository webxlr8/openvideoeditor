import { useRef, useState } from "react";
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
import { Icon, type IconName } from "./icons";
import { Dropdown } from "./Menu";
import { formatShort, formatTimecode, parseTimecode } from "./timecode";
import { Viewer } from "./Viewer";

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

type EditResult = Promise<{ status: "ok"; data: ProjectView_Serialize } | { status: "error"; error: AppError }>;

/** Timeline time of the playhead: the previewed clip's start plus how far into its source the viewer is. */
export function playheadSeconds(item: ItemView_Serialize | undefined, playback: PlaybackState, previewing: boolean) {
  if (!item || !previewing) return 0;
  const fps = playback.frameRate ?? 30;
  const t = (item.start ?? 0) + playback.frame / fps - (item.trimStart ?? 0);
  return Math.min(Math.max(t, item.start ?? 0), item.end ?? 0);
}

export function Editor({
  project,
  selected,
  onSelect,
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
  selected: string | null;
  onSelect: (item: ItemView_Serialize | null, offsetSeconds?: number) => void;
  onEdit: (p: EditResult) => void;
}) {
  const [panel, setPanel] = useState("media");
  const toggle = (key: keyof Layout) => setLayout({ ...layout, [key]: !layout[key] });
  const item = project?.items.find((i) => i.id === selected);
  const itemMedia = project?.media.find((m) => m.id === item?.mediaId);
  const previewing = !!(item && itemMedia && media && media.path === itemMedia.path);
  const playhead = playheadSeconds(item, playback, previewing);
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
                  if (panel === r.id) setLayout({ ...layout, panelOpen: !layout.panelOpen });
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
          {layout.panelOpen && (
            <SidePanel
              id={panel}
              project={project}
              onImport={onImport}
              onCollapse={() => toggle("panelOpen")}
              onPick={(m) => {
                const it = project?.items.find((i) => i.mediaId === m.id);
                if (it) onSelect(it);
              }}
            />
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
        />
        {layout.timelineOpen ? (
          <Timeline project={project} selected={selected} playhead={playhead} onSelect={onSelect} onEdit={onEdit} />
        ) : (
          <ScrubRow playback={playback} onTransport={onTransport} />
        )}
      </div>
      {layout.chatOpen && <ChatPanel onClose={() => toggle("chatOpen")} />}
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
  onImport,
  onCollapse,
  onPick,
}: {
  id: string;
  project: ProjectView_Serialize | null;
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
      ) : (
        <p className="muted panel-placeholder">{label} tools arrive in a later version.</p>
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
}: {
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
  layout: Layout;
  toggle: (k: keyof Layout) => void;
  project: ProjectView_Serialize | null;
  canSplit: boolean;
  onEdit: (p: EditResult) => void;
  onSplit: () => void;
}) {
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
        <button className="icon-btn" title="Split the selected clip at the playhead" disabled={!canSplit} data-testid="split" onClick={onSplit}>
          <Icon name="scissors" size={16} />
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

const HEADER_W = 188;
const TRACK_ICON: Record<string, IconName> = { video: "media", caption: "captions", audio: "audio", adjustment: "effects", layout: "layout" };

function Timeline({
  project,
  selected,
  playhead,
  onSelect,
  onEdit,
}: {
  project: ProjectView_Serialize | null;
  selected: string | null;
  playhead: number;
  onSelect: (item: ItemView_Serialize | null, offsetSeconds?: number) => void;
  onEdit: (p: EditResult) => void;
}) {
  const lanes = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ id: string; dx: number } | null>(null);
  const items = project?.items ?? [];
  const duration = Math.max(10, (project?.durationSeconds ?? 0) * 1.15, playhead + 1);
  const step = duration > 600 ? 60 : duration > 240 ? 30 : duration > 80 ? 10 : duration > 30 ? 5 : duration > 12 ? 2 : 1;
  const ticks: number[] = [];
  for (let s = 0; s <= duration; s += step) ticks.push(s);
  const laneWidth = () => Math.max(1, (lanes.current?.getBoundingClientRect().width ?? 1000) - HEADER_W);
  const pct = (t: number) => `${(t / duration) * 100}%`;

  const seekAt = (clientX: number) => {
    const r = lanes.current?.getBoundingClientRect();
    if (!r) return;
    const t = Math.max(0, ((clientX - r.left - HEADER_W) / laneWidth()) * duration);
    // The topmost visual clip under the playhead is previewed (sequence playback arrives with the compositor).
    const order = new Map(project?.tracks.map((tr, i) => [tr.id, i]));
    const hit = items
      .filter((i) => i.mediaId && (i.start ?? 0) <= t && t < (i.end ?? 0))
      .sort((a, b) => (order.get(a.trackId) ?? 0) - (order.get(b.trackId) ?? 0))[0];
    if (hit) onSelect(hit, t - (hit.start ?? 0));
  };

  const startDrag = (e: React.PointerEvent, item: ItemView_Serialize) => {
    e.stopPropagation();
    onSelect(item);
    const x0 = e.clientX;
    let dx = 0;
    const move = (ev: PointerEvent) => {
      dx = ev.clientX - x0;
      if (Math.abs(dx) > 3) setDrag({ id: item.id, dx });
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      if (Math.abs(dx) > 3) {
        const start = Math.max(0, (item.start ?? 0) + (dx / laneWidth()) * duration);
        onEdit(commands.moveItem(item.id, Math.round(start * 100) / 100, null));
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <div className="timeline" data-testid="timeline" ref={lanes}>
      <div className="ruler" onPointerDown={(e) => seekAt(e.clientX)} data-testid="ruler">
        <div className="ruler-head small">{fmtTime(playhead)}</div>
        <div className="ruler-ticks">
          {ticks.map((s) => (
            <span key={s} style={{ left: pct(s) }}>
              {fmtTick(s)}
            </span>
          ))}
        </div>
      </div>
      <div className="tracks">
        {(project?.tracks ?? []).map((t) => (
          <div className={`track${t.visible ? "" : " track-hidden"}`} key={t.id} data-testid={`track-${t.name}`}>
            <div className="track-head">
              <Icon name={TRACK_ICON[t.kind] ?? "media"} size={14} />
              <span>{t.name}</span>
              <span className="track-tools">
                {t.kind !== "caption" && (
                  <button
                    className={`tiny${t.muted ? " on" : ""}`}
                    title={t.muted ? "Unmute track" : "Mute track"}
                    onClick={() => onEdit(commands.setTrackFlag(t.id, "muted", !t.muted))}
                  >
                    M
                  </button>
                )}
                <button
                  className={`tiny${t.visible ? "" : " on"}`}
                  aria-label="Visibility"
                  title={t.visible ? "Hide track" : "Show track"}
                  onClick={() => onEdit(commands.setTrackFlag(t.id, "visible", !t.visible))}
                >
                  <Icon name="eye" size={14} />
                </button>
              </span>
            </div>
            <div className="track-lane" onPointerDown={(e) => (e.target === e.currentTarget ? seekAt(e.clientX) : undefined)}>
              {items
                .filter((i) => i.trackId === t.id)
                .map((i) => {
                  const thumb = project?.media.find((m) => m.id === i.mediaId)?.thumbnail;
                  const offset = drag?.id === i.id ? drag.dx : 0;
                  return (
                    <div
                      key={i.id}
                      className={`clip kind-${i.kind}${i.id === selected ? " selected" : ""}${i.pending ? " pending" : ""}`}
                      style={{
                        left: `calc(${pct(i.start ?? 0)} + ${offset}px)`,
                        width: pct((i.end ?? 0) - (i.start ?? 0)),
                        backgroundImage: thumb ? `url(${thumb})` : undefined,
                      }}
                      title={`${i.name} · ${fmtTime(i.start ?? 0)}–${fmtTime(i.end ?? 0)}`}
                      data-testid="clip"
                      onPointerDown={(e) => startDrag(e, i)}
                    >
                      <span className="clip-label">{i.name}</span>
                    </div>
                  );
                })}
            </div>
          </div>
        ))}
        {!project?.items.length && <div className="timeline-empty muted">Import media to start your timeline.</div>}
      </div>
      <div className="playhead" style={{ left: `calc(${HEADER_W}px + (100% - ${HEADER_W}px) * ${playhead / duration})` }} data-testid="playhead" />
    </div>
  );
}

function fmtTime(t: number) {
  const ms = Math.round(t * 1000);
  const s = Math.floor(ms / 1000);
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}.${String(Math.floor((ms % 1000) / 10)).padStart(2, "0")}`;
}

function fmtTick(s: number) {
  return s >= 60 ? `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}` : `${s}s`;
}

function ChatPanel({ onClose }: { onClose: () => void }) {
  return (
    <aside className="chat" data-testid="chat">
      <header className="chat-header">
        <button className="pill">New conversation</button>
        <span className="chat-header-tools">
          <button className="icon-btn" title="History (later)" disabled>
            <Icon name="history" size={16} />
          </button>
          <button className="icon-btn" title="Hide assistant" onClick={onClose} data-testid="close-chat">
            <Icon name="close" size={16} />
          </button>
        </span>
      </header>
      <div className="chat-empty">
        <Icon name="spark" size={34} />
        <p>Ask for a cut, captions, or a fresh idea.</p>
      </div>
      <div className="composer">
        <textarea placeholder="Describe an edit…" rows={3} />
        <div className="composer-row">
          <button className="icon-btn" title="Attach (later)" disabled>
            <Icon name="attach" size={16} />
          </button>
          <span className="grow" />
          <Dropdown
            testId="agent-picker"
            direction="up"
            label="Your agent"
            items={[
              { id: "api", label: "Anthropic API key" },
              { id: "cli", label: "Local coding agent" },
            ]}
          />
          <button className="send" title="Agents arrive in a later version" disabled>
            <Icon name="send" size={16} />
          </button>
        </div>
      </div>
    </aside>
  );
}
