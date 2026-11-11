import { useRef, useState } from "react";
import type { AppError, MediaInfo_Serialize, PlaybackState, TransportCommand, ViewerBackend } from "./bindings";
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

export function Editor({
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
}) {
  const [panel, setPanel] = useState("media");
  const toggle = (key: keyof Layout) => setLayout({ ...layout, [key]: !layout[key] });

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
            <SidePanel id={panel} media={media} onImport={onImport} onCollapse={() => toggle("panelOpen")} />
          )}
          <Viewer
            media={media}
            frame={playback.frame}
            backend={backend}
            visible={layout.viewerOpen}
            diagnosticsOpen={diagnosticsOpen}
            onError={onError}
          />
        </div>
        <TransportBar playback={playback} onTransport={onTransport} layout={layout} toggle={toggle} />
        {layout.timelineOpen ? (
          <Timeline media={media} playback={playback} onTransport={onTransport} />
        ) : (
          <ScrubRow playback={playback} onTransport={onTransport} />
        )}
      </div>
      {layout.chatOpen && <ChatPanel onClose={() => toggle("chatOpen")} />}
    </div>
  );
}

function SidePanel({
  id,
  media,
  onImport,
  onCollapse,
}: {
  id: string;
  media: MediaInfo_Serialize | null;
  onImport: () => void;
  onCollapse: () => void;
}) {
  const label = RAIL.find((r) => r.id === id)?.label ?? "";
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
              <input placeholder="Search media…" />
            </label>
            <button className="btn" onClick={onImport} data-testid="import">
              <Icon name="plus" size={15} /> Import
            </button>
          </div>
          <div className="media-grid">
            {media ? (
              <div className="media-card" title={media.path}>
                <div className="media-thumb">
                  <span className="badge">{formatShort(media.totalFrames, media.frameRate)}</span>
                </div>
                <div className="media-name">{media.name}</div>
              </div>
            ) : (
              <p className="muted">No media yet. Import a file or drop it on the window.</p>
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
}: {
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
  layout: Layout;
  toggle: (k: keyof Layout) => void;
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
        <button className="icon-btn" title="Undo (later)" disabled>
          <Icon name="undo" size={16} />
        </button>
        <button className="icon-btn" title="Redo (later)" disabled>
          <Icon name="redo" size={16} />
        </button>
        <button className="icon-btn" title="Split (later)" disabled>
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

const TRACKS = [
  { id: "text", name: "Text", icon: "text" as IconName },
  { id: "captions", name: "Captions", icon: "captions" as IconName },
  { id: "overlay", name: "Overlay", icon: "media" as IconName },
  { id: "main", name: "Main", icon: "media" as IconName },
];
const HEADER_W = 188;

function Timeline({
  media,
  playback,
  onTransport,
}: {
  media: MediaInfo_Serialize | null;
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
}) {
  const { el, onPointerDown } = useScrub(playback, onTransport);
  const total = Math.max(1, playback.totalFrames);
  const pct = playback.totalFrames > 1 ? playback.frame / (playback.totalFrames - 1) : 0;
  const seconds = Math.max(1, Math.round(total / (playback.frameRate ?? 30)));
  const step = seconds > 120 ? 30 : seconds > 40 ? 10 : 5;
  const ticks = [];
  for (let s = 0; s <= seconds; s += step) ticks.push(s);
  return (
    <div className="timeline" data-testid="timeline" ref={el}>
      <div className="ruler" onPointerDown={(e) => onPointerDown(e, HEADER_W)} data-testid="ruler">
        <div className="ruler-head small">{formatTimecode(playback.frame, playback.frameRate)}</div>
        <div className="ruler-ticks">
          {ticks.map((s) => (
            <span key={s} style={{ left: `${(s / seconds) * 100}%` }}>
              {s}s
            </span>
          ))}
        </div>
      </div>
      {TRACKS.map((t) => (
        <div className="track" key={t.id}>
          <div className="track-head">
            <Icon name={t.icon} size={14} />
            <span>{t.name}</span>
            <span className="track-tools">
              {t.id !== "captions" && <button className="tiny">M</button>}
              <button className="tiny" aria-label="Visibility">
                <Icon name="eye" size={14} />
              </button>
            </span>
          </div>
          <div className="track-lane">
            {t.id === "main" && media && (
              <div className="clip" style={{ left: 0, width: "100%" }} title={media.name}>
                {media.name}
              </div>
            )}
          </div>
        </div>
      ))}
      <div className="playhead" style={{ left: `calc(${HEADER_W}px + (100% - ${HEADER_W}px) * ${pct})` }} data-testid="playhead" />
    </div>
  );
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
