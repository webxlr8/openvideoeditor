// The media panel (add-semantic-media-search D8): the project's media by folder, and one search field over what the
// footage shows (Moments), what is said (Spoken), the timeline (Timelines) and names, folders and tags (Files).
// Results come from the same `search_media` registry tool agents use. Moment and Spoken results drag onto a track
// as a clip with the result's source range, and can be saved as subclips (both one revision).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  MediaIndexStatus,
  MediaView_Serialize,
  ProjectView_Serialize,
} from "../bindings";
import { commands, skimUrl } from "../bridge";
import { Icon } from "../icons";
import { PanelHint } from "../onboarding/PanelHint";
import type { PanelProps } from "../Panels";
import { SkimThumb, tileStyle } from "../SkimThumb";
import { sizeText } from "../storage/ModelsSection";
import "./media.css";
import { AskAssistant } from "../chat/AskAssistant";

/** Typing pauses this long before a search runs. */
export const DEBOUNCE_MS = 250;
/** The index pill refreshes this often while files are being indexed. */
const STATUS_POLL_MS = 3000;
/** The drag payload type for a source range dropped on a track. */
export const CLIP_DRAG_TYPE = "application/x-ove-clip";

export const SCOPES = [
  { id: "all", label: "All" },
  { id: "moments", label: "Moments" },
  { id: "spoken", label: "Spoken" },
  { id: "timelines", label: "Timelines" },
  { id: "files", label: "Files" },
] as const;
export type Scope = (typeof SCOPES)[number]["id"];

/** A source range dragged from the panel: what the timeline places. */
export interface ClipDrag {
  mediaId: string;
  name: string;
  kind: string;
  /** Source seconds. */
  start: number;
  end: number;
}

interface MomentHit {
  media: string;
  name: string;
  shot: [number, number];
  best: number;
  in: number;
  out: number;
  confidence: number;
  clip: string;
}
interface SpokenHit {
  media: string;
  name: string;
  text: string;
  in: number;
  out: number;
  clip: string;
}
interface TimelineHit {
  item: string;
  type: string;
  start: number;
  end: number;
  name: string;
  text: string;
}
interface FileHit {
  media: string;
  name: string;
  folder: string | null;
  tags: string[];
}
export interface SearchResult {
  moments?: MomentHit[];
  spoken?: SpokenHit[];
  timelines?: TimelineHit[];
  files?: FileHit[];
  index?: {
    visual?: string;
    indexed?: number;
    total?: number;
    downloadBytes?: number;
    untranscribed?: number;
  };
}

const secs = (t: number | null) => {
  t = t ?? 0;
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(1).padStart(4, "0")}`;
};

/** The index pill's text and whether it links to the background tasks. */
export function pillText(s: MediaIndexStatus | null): { text: string; tasks: boolean } | null {
  if (!s) return null;
  if (!s.modelInstalled)
    return { text: `Visual search not installed · ${sizeText(s.downloadBytes)}`, tasks: false };
  if (s.total === 0) return null;
  if (s.indexed < s.total)
    return { text: `Indexing ${s.indexed} of ${s.total}`, tasks: true };
  return { text: `${s.total} indexed`, tasks: false };
}

function dragProps(d: ClipDrag) {
  return {
    draggable: true,
    onDragStart: (e: React.DragEvent) => {
      e.dataTransfer.setData(CLIP_DRAG_TYPE, JSON.stringify(d));
      e.dataTransfer.setData("text/plain", `${d.name} ${d.start}–${d.end}`);
      e.dataTransfer.effectAllowed = "copy";
    },
  };
}

/** A frame of a media file at `t` seconds, from its skim sheet (no decoding). */
function Frame({ media, t }: { media?: MediaView_Serialize; t: number }) {
  const skim = media?.skim;
  if (!skim || !media?.durationSeconds) {
    return media?.thumbnail ? (
      <img className="ms-frame" src={media.thumbnail} alt="" draggable={false} />
    ) : (
      <span className="ms-frame ms-frame-empty">
        <Icon name="media" size={18} />
      </span>
    );
  }
  const i = Math.min(skim.count - 1, Math.max(0, Math.floor((t / media.durationSeconds) * skim.count)));
  return (
    <span
      className="ms-frame"
      style={{ backgroundImage: `url("${skimUrl(skim.contentId)}")`, ...tileStyle(skim, i) }}
    />
  );
}

export function MediaPanel({
  project,
  panelProps,
  onImport,
  onImportTimeline,
  onPick,
}: {
  project: ProjectView_Serialize | null;
  panelProps: PanelProps | null;
  onImport: () => void;
  onImportTimeline?: () => void;
  onPick: (m: MediaView_Serialize) => void;
}) {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [scope, setScope] = useState<Scope>("all");
  const [result, setResult] = useState<SearchResult | null>(null);
  /** The query the shown results answer; while it differs from the field, a search is under way. */
  const [searched, setSearched] = useState("");
  const [status, setStatus] = useState<MediaIndexStatus | null>(null);
  const [folder, setFolder] = useState<string | null>(null);
  const [confirmInstall, setConfirmInstall] = useState(false);
  const seq = useRef(0);
  const onError = panelProps?.onError;
  const rev = project?.rev;

  useEffect(() => {
    const t = setTimeout(() => setDebounced(query.trim()), DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query]);

  // The index pill: on open, after each revision, and every few seconds while files are being indexed.
  const indexing = !!status && status.modelInstalled && status.indexed < status.total;
  useEffect(() => {
    if (!project) return;
    let live = true;
    const read = () => {
      commands
        .mediaIndexStatus()
        .then((r) => {
          if (live && r.status === "ok") setStatus(r.data);
        })
        .catch(() => undefined);
    };
    read();
    const t = indexing ? setInterval(read, STATUS_POLL_MS) : undefined;
    return () => {
      live = false;
      if (t) clearInterval(t);
    };
  }, [project, rev, indexing]);

  useEffect(() => {
    // An empty field shows the media instead of results (see `searchingNow`); nothing to fetch.
    if (!debounced || !project) return;
    const n = ++seq.current;
    commands.mediaSearch(JSON.stringify({ query: debounced, scope })).then((r) => {
      if (n !== seq.current) return;
      setSearched(debounced);
      if (r.status === "ok") setResult(JSON.parse(r.data) as SearchResult);
      else {
        setResult(null);
        onError?.(r.error);
      }
    }).catch(() => undefined);
  }, [debounced, scope, project, rev, onError]);

  const byId = useMemo(() => new Map((project?.media ?? []).map((m) => [m.id, m])), [project]);
  const folders = useMemo(() => {
    const set = new Set<string>();
    for (const m of project?.media ?? []) if (m.folder) set.add(m.folder.split("/")[0]);
    return [...set].sort();
  }, [project]);
  const used = new Set(project?.items.map((i) => i.mediaId));
  const shown = (project?.media ?? []).filter(
    (m) => !folder || m.folder === folder || m.folder?.startsWith(`${folder}/`),
  );

  const organize = useCallback(
    (ops: unknown[]) => panelProps?.onEdit(commands.mediaOrganize(JSON.stringify(ops))),
    [panelProps],
  );
  const saveSubclip = (h: { media: string; in: number; out: number; name: string }) =>
    organize([{ op: "saveSubclip", media: h.media, in: h.in, out: h.out, name: `${h.name} ${secs(h.in)}` }]);

  const install = () => {
    setConfirmInstall(false);
    // Both downloads were agreed to by this click: the runtime first (when missing), then the model.
    commands
      .installLocalModel("onnxruntime")
      .then(() => commands.installLocalModel("siglip2-base"))
      .then((r) => {
        if (r.status === "error") onError?.(r.error);
      })
      .catch(() => undefined);
  };

  const pill = pillText(status);
  const want = (s: Scope) => scope === "all" || scope === s;
  const searchingNow = !!debounced;

  return (
    <div className="media-panel" data-testid="media-panel">
      <div className="panel-search">
        <label className="search">
          <Icon name="search" size={15} />
          <input
            placeholder="Search footage, speech, names…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            data-testid="media-search"
            aria-label="Search media"
          />
        </label>
        <button className="btn" onClick={onImport} data-testid="import">
          <Icon name="plus" size={15} /> Import
        </button>
        {onImportTimeline && (
          <button
            className="icon-btn"
            onClick={onImportTimeline}
            title="Import a timeline (FCPXML, Premiere XML, EDL, OpenTimelineIO)"
            aria-label="Import timeline"
            data-testid="import-timeline"
          >
            <Icon name="timeline" size={15} />
          </button>
        )}
      </div>
      {project && project.media.length > 0 && <AskAssistant panel="media" project={project} selection={[]} />}
      <div className="ms-bar">
        {searchingNow && (
          <div className="ms-chips" role="group" aria-label="Search scope">
            {SCOPES.map((s) => (
              <button
                key={s.id}
                className={`ms-chip${scope === s.id ? " on" : ""}`}
                aria-pressed={scope === s.id}
                onClick={() => setScope(s.id)}
                data-testid={`scope-${s.id}`}
              >
                {s.label}
              </button>
            ))}
          </div>
        )}
        {!searchingNow && folders.length > 0 && (
          <div className="ms-chips" role="group" aria-label="Folder">
            <button className={`ms-chip${folder === null ? " on" : ""}`} onClick={() => setFolder(null)}>
              All media
            </button>
            {folders.map((f) => (
              <button
                key={f}
                className={`ms-chip${folder === f ? " on" : ""}`}
                onClick={() => setFolder(f)}
                data-testid={`folder-${f}`}
              >
                <Icon name="projects" size={12} /> {f}
              </button>
            ))}
          </div>
        )}
        {pill && (
          <button
            className={`ms-pill${pill.tasks ? " busy" : ""}`}
            data-testid="index-pill"
            onClick={() =>
              pill.tasks
                ? window.dispatchEvent(new CustomEvent("ove:open-tasks"))
                : !status?.modelInstalled && setConfirmInstall(true)
            }
            title={pill.tasks ? "Show background tasks" : undefined}
          >
            {pill.text}
          </button>
        )}
      </div>
      {confirmInstall && status && (
        <div className="ms-install" role="dialog" aria-label="Install visual search" data-testid="install-offer">
          <p>
            Visual search finds shots by what they show, on this computer. It needs the model{" "}
            <strong data-testid="install-model">{status.modelName}</strong> (licence{" "}
            <span data-testid="install-licence">{status.licence}</span>), a {sizeText(status.downloadBytes)} download
            from <span className="mono">{status.host}</span>, and the inference runtime if it is not installed yet.
          </p>
          <div className="ms-install-actions">
            <button className="btn primary" onClick={install} data-testid="install-confirm">
              Download {sizeText(status.downloadBytes)}
            </button>
            <button className="btn" onClick={() => setConfirmInstall(false)}>
              Not now
            </button>
          </div>
        </div>
      )}

      {searchingNow ? (
        <div className="ms-results" data-testid="media-results" aria-busy={searched !== debounced}>
          {want("moments") && (
            <section className="ms-section" data-testid="section-moments">
              <h3>Moments</h3>
              {result?.index?.visual === "notInstalled" ? (
                <p className="muted small" data-testid="moments-not-installed">
                  Visual search is not installed ({sizeText(result.index.downloadBytes ?? 0)}).{" "}
                  <button className="link" onClick={() => setConfirmInstall(true)}>
                    Install…
                  </button>
                </p>
              ) : (result?.moments ?? []).length === 0 ? (
                <p className="muted small" data-testid="moments-empty">
                  No visual matches.
                </p>
              ) : (
                <ul className="ms-list">
                  {result!.moments!.map((h) => (
                    <li
                      key={`${h.media}@${h.best}`}
                      className="ms-hit"
                      data-testid="moment"
                      {...dragProps({ mediaId: h.media, name: h.name, kind: byId.get(h.media)?.kind ?? "video", start: h.in, end: h.out })}
                    >
                      <Frame media={byId.get(h.media)} t={h.best} />
                      <span className="ms-hit-text">
                        <span className="ms-hit-name">{h.name}</span>
                        <span className="muted small">
                          {secs(h.in)}–{secs(h.out)} · {Math.round(h.confidence * 100)}%
                        </span>
                      </span>
                      <button className="icon-btn" title="Save as subclip" onClick={() => saveSubclip(h)} data-testid="save-subclip">
                        <Icon name="plus" size={13} />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}
          {want("spoken") && (
            <section className="ms-section" data-testid="section-spoken">
              <h3>Spoken</h3>
              {(result?.spoken ?? []).length === 0 ? (
                <p className="muted small">No spoken matches.</p>
              ) : (
                <ul className="ms-list">
                  {result!.spoken!.map((h) => (
                    <li
                      key={`${h.media}@${h.in}`}
                      className="ms-hit"
                      data-testid="spoken"
                      {...dragProps({ mediaId: h.media, name: h.name, kind: byId.get(h.media)?.kind ?? "video", start: h.in, end: h.out })}
                    >
                      <Frame media={byId.get(h.media)} t={h.in} />
                      <span className="ms-hit-text">
                        <span className="ms-hit-name">“{h.text}”</span>
                        <span className="muted small">
                          {h.name} · {secs(h.in)}–{secs(h.out)}
                        </span>
                      </span>
                      <button className="icon-btn" title="Save as subclip" onClick={() => saveSubclip(h)}>
                        <Icon name="plus" size={13} />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              {(result?.index?.untranscribed ?? 0) > 0 && (
                <p className="muted small" data-testid="untranscribed">
                  {result!.index!.untranscribed} file{result!.index!.untranscribed === 1 ? " is" : "s are"} not
                  transcribed.{" "}
                  <button
                    className="link"
                    onClick={() => window.dispatchEvent(new CustomEvent("ove:open-panel", { detail: "transcript" }))}
                  >
                    Transcribe
                  </button>
                </p>
              )}
            </section>
          )}
          {want("timelines") && (
            <section className="ms-section" data-testid="section-timelines">
              <h3>Timelines</h3>
              {(result?.timelines ?? []).length === 0 ? (
                <p className="muted small">No timeline matches.</p>
              ) : (
                <ul className="ms-list">
                  {result!.timelines!.map((h) => (
                    <li key={h.item} className="ms-hit" data-testid="timeline-hit">
                      <button className="ms-hit-text link" onClick={() => panelProps?.onSeek?.(h.start)}>
                        <span className="ms-hit-name">{h.name || h.text || h.item}</span>
                        <span className="muted small">
                          {h.type} · {secs(h.start)}–{secs(h.end)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}
          {want("files") && (
            <section className="ms-section" data-testid="section-files">
              <h3>Files</h3>
              {(result?.files ?? []).length === 0 ? (
                <p className="muted small">No files match.</p>
              ) : (
                <ul className="ms-list">
                  {result!.files!.map((h) => {
                    const m = byId.get(h.media);
                    return (
                      <li key={h.media} className="ms-hit" data-testid="file-hit">
                        <button className="ms-hit-text link" onClick={() => m && onPick(m)}>
                          <span className="ms-hit-name">{h.name}</span>
                          <span className="muted small">
                            {[h.folder, ...h.tags].filter(Boolean).join(" · ")}
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>
          )}
        </div>
      ) : (
        <div className="media-grid">
          {shown.map((m) => (
            <div key={m.id} className="media-entry">
              <button
                className="media-card"
                title={m.path}
                onClick={() => onPick(m)}
                data-testid="media-card"
                {...dragProps({ mediaId: m.id, name: m.name, kind: m.kind, start: 0, end: m.durationSeconds ?? 5 })}
              >
                <div className="media-thumb">
                  {m.thumbnail || m.skim ? (
                    <SkimThumb
                      thumbnail={m.thumbnail}
                      skim={m.skim}
                      video={m.kind !== "audio"}
                      onRequest={() => {
                        commands.requestSkim(m.id).catch(() => undefined);
                      }}
                    />
                  ) : (
                    <Icon name={m.kind === "audio" ? "audio" : "media"} size={22} />
                  )}
                  <span className="badge">{m.durationSeconds != null ? secs(m.durationSeconds) : ""}</span>
                  {m.status !== "online" && <span className="badge warn">{m.status}</span>}
                  {used.has(m.id) && <span className="badge added">On timeline</span>}
                </div>
                <div className="media-name">
                  <Icon name={m.kind === "audio" ? "audio" : "play"} size={12} /> {m.name}
                </div>
                {m.tags.length > 0 && <div className="ms-tags muted small">{m.tags.join(" · ")}</div>}
                {m.timecode && (
                  <div className="media-tc" data-testid="media-timecode" title="Source timecode">
                    TC {m.timecode}
                  </div>
                )}
              </button>
              {m.subclips.map((s) => (
                <div
                  key={s.id}
                  className="ms-subclip"
                  data-testid="subclip"
                  {...dragProps({ mediaId: m.id, name: s.name, kind: m.kind, start: s.start ?? 0, end: s.end ?? 0 })}
                >
                  <Icon name="scissors" size={11} /> {s.name}{" "}
                  <span className="muted small">
                    {secs(s.start)}–{secs(s.end)}
                  </span>
                </div>
              ))}
            </div>
          ))}
          {shown.length === 0 &&
            (folder ? (
              <p className="muted">This folder is empty.</p>
            ) : (
              <PanelHint panel="media" action="Import media" onAction={onImport}>
                The video, sound and picture files of this project are kept here. You can also drop files on the
                window.
              </PanelHint>
            ))}
        </div>
      )}
    </div>
  );
}
