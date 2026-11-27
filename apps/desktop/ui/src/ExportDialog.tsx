import { useEffect, useState } from "react";
import type {
  AppError,
  ExportCheck,
  ExportForm,
  ExportJobView,
  ExportOptions,
  ExportPresetView,
  SavedRange,
  TimelineExportReport,
} from "./bindings";
import { commands } from "./bridge";
import { Icon } from "./icons";
import { fire, runCommand } from "./notify";
import "./export.css";

export type PreviewMode = "timeline" | "source";

/** Switches the viewer between the composited timeline and the selected clip's source. */
export function PreviewModeToggle({
  mode,
  onChange,
}: {
  mode: PreviewMode;
  onChange: (m: PreviewMode) => void;
}) {
  return (
    <div
      className="xp-seg"
      role="radiogroup"
      aria-label="Viewer shows"
      data-testid="preview-mode"
    >
      {(["timeline", "source"] as const).map((m) => (
        <button
          key={m}
          role="radio"
          aria-checked={mode === m}
          className={mode === m ? "on" : ""}
          title={
            m === "timeline"
              ? "Play the whole timeline as it will export"
              : "Play the selected clip's source file"
          }
          onClick={() => onChange(m)}
          data-testid={`preview-${m}`}
        >
          {m === "timeline" ? "Timeline" : "Source"}
        </button>
      ))}
    </div>
  );
}

/** The timeline's reference frame size for an aspect ratio (matches the render engine). */
export function referenceSize(aspect: string): [number, number] {
  const sizes: Record<string, [number, number]> = {
    "9:16": [1080, 1920],
    "1:1": [1080, 1080],
    "4:5": [1080, 1350],
    "4:3": [1440, 1080],
    "3:4": [1080, 1440],
  };
  return sizes[aspect] ?? [1920, 1080];
}


/** Timeline exchange formats (ids match the `ove timeline export --format` values). */
const TIMELINE_FORMATS = [
  { id: "fcpxml", label: "FCPXML", note: "Final Cut Pro, DaVinci Resolve" },
  {
    id: "xml",
    label: "XML",
    note: "Premiere Pro, DaVinci Resolve, Final Cut Pro 7",
  },
  {
    id: "edl",
    label: "EDL",
    note: "CMX3600 — main video track and first audio track",
  },
  { id: "otio", label: "OTIO", note: "OpenTimelineIO" },
];
const TIMELINE_RATES = [
  { label: "23.976", fps: 23.976 },
  { label: "24", fps: 24 },
  { label: "25", fps: 25 },
  { label: "29.97", fps: 29.97 },
  { label: "30", fps: 30 },
  { label: "60", fps: 60 },
];

/** "What won't carry over": the items a format leaves out and the properties it doesn't keep. */
export function CarryOverList({ report }: { report: TimelineExportReport }) {
  return (
    <div className="xp-report" data-testid="timeline-report">
      <h4>What won't carry over</h4>
      <ul>
        {report.omitted.map((o) => (
          <li key={`${o.kind}|${o.reason}`} data-testid="timeline-omitted">
            <strong>
              {o.kind.charAt(0).toUpperCase() + o.kind.slice(1)} ({o.count})
            </strong>{" "}
            <span className="muted">— {o.reason}</span>
          </li>
        ))}
        {report.notCarried.length > 0 && (
          <li data-testid="timeline-not-carried">
            <strong>Not kept on the clips that are written:</strong>{" "}
            {report.notCarried.join(", ")}
          </li>
        )}
      </ul>
      <p className="muted small">
        Your project keeps all of it; only the exported file goes without.
      </p>
    </div>
  );
}

const lossless = (r: TimelineExportReport) =>
  r.omitted.length === 0 && r.notCarried.length === 0;

/** Export of the edit as a timeline file for another editor. */
function TimelineExport({
  onClose,
  onError,
}: {
  onClose: () => void;
  onError: (e: AppError) => void;
}) {
  const [format, setFormat] = useState("fcpxml");
  const [fps, setFps] = useState(30);
  const [saved, setSaved] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** The report for the chosen format, shown before writing when something won't carry over. */
  const [check, setCheck] = useState<TimelineExportReport | null>(null);
  const write = async () => {
    setBusy(true);
    try {
      const r = await commands.exportTimeline(format, fps, null, false);
      if (r.status === "error") onError(r.error);
      else if (r.data.path) setSaved(r.data.path);
    } finally {
      setBusy(false);
    }
  };
  const start = async () => {
    setBusy(true);
    let report: TimelineExportReport;
    try {
      const r = await commands.exportTimeline(format, fps, null, true);
      if (r.status === "error") return onError(r.error);
      report = r.data.report;
    } finally {
      setBusy(false);
    }
    if (lossless(report)) await write();
    else setCheck(report);
  };
  if (saved)
    return (
      <div className="xp-result" data-testid="timeline-export-done">
        <p>
          Saved <strong>{saved.split(/[\\/]/).pop()}</strong>
        </p>
        <div className="xp-actions">
          <button
            className="btn"
            onClick={() =>
              runCommand(commands.revealPath(saved), undefined, onError)
            }
          >
            Show in folder
          </button>
          <button className="btn primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    );
  const label = TIMELINE_FORMATS.find((f) => f.id === format)?.label;
  if (check)
    return (
      <>
        <p className="small">
          The {label} file will leave some of this project behind:
        </p>
        <CarryOverList report={check} />
        <div className="xp-actions">
          <button className="btn" onClick={() => setCheck(null)}>
            Back
          </button>
          <button
            className="btn primary"
            onClick={() => fire(write(), onError)}
            disabled={busy}
            data-testid="timeline-export-continue"
          >
            Continue…
          </button>
        </div>
      </>
    );
  const note = TIMELINE_FORMATS.find((f) => f.id === format)?.note;
  return (
    <>
      <div className="xp-field">
        <span>Format</span>
        <div className="xp-seg" data-testid="timeline-formats">
          {TIMELINE_FORMATS.map((f) => (
            <button
              key={f.id}
              className={format === f.id ? "on" : ""}
              onClick={() => setFormat(f.id)}
              title={f.note}
            >
              {f.label}
            </button>
          ))}
        </div>
        <span className="muted small">{note}</span>
      </div>
      <div className="xp-field">
        <span>Frame rate</span>
        <div className="xp-seg">
          {TIMELINE_RATES.map((r) => (
            <button
              key={r.label}
              className={fps === r.fps ? "on" : ""}
              onClick={() => setFps(r.fps)}
            >
              {r.label}
            </button>
          ))}
        </div>
      </div>
      <p className="muted small">
        Clips keep their trims, speed changes and tracks; media stays where it
        is and the file links to it. Before anything is saved you'll see what
        this format can't carry over.
      </p>
      <div className="xp-actions">
        <button className="btn" onClick={onClose}>
          Close
        </button>
        <button
          className="btn primary"
          onClick={() => fire(start(), onError)}
          disabled={busy}
          data-testid="timeline-export-start"
        >
          Export timeline…
        </button>
      </div>
    </>
  );
}

// ---- The export queue: opening the dialog and the Exports panel from anywhere, and the jobs list. ----

/** Asks the app to open the export dialog, for the whole timeline or a range (seconds). */
export function openExport(range?: [number, number] | null) {
  window.dispatchEvent(
    new CustomEvent("ove:open-export", { detail: { range: range ?? null } }),
  );
}

/** Calls `f` when something asks for the export dialog; returns the unsubscribe. */
export function onOpenExport(
  f: (range: [number, number] | null) => void,
): () => void {
  const h = (e: Event) =>
    f((e as CustomEvent<{ range: [number, number] | null }>).detail?.range ?? null);
  window.addEventListener("ove:open-export", h);
  return () => window.removeEventListener("ove:open-export", h);
}

/** Asks the editor to show the Exports panel (the background-tasks popover links here). */
export function openExportsPanel() {
  window.dispatchEvent(new CustomEvent("ove:open-panel", { detail: "exports" }));
}

export const jobActive = (j: ExportJobView) =>
  j.status === "queued" || j.status === "running";

/** The export queue, read from the queue files: every 0.5 s while a job is queued or running, else every 3 s. */
export function useExportJobs(onError?: (e: AppError) => void) {
  const [jobs, setJobs] = useState<ExportJobView[] | null>(null);
  const [tick, setTick] = useState(0);
  const active = (jobs ?? []).some(jobActive);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      const r = await commands.exportJobs();
      if (!live) return;
      if (r.status === "error") onError?.(r.error);
      else setJobs(r.data);
    };
    const loop = () => {
      load()
        .finally(() => {
          if (live) timer = setTimeout(loop, active ? 500 : 3000);
        })
        .catch(() => undefined);
    };
    loop();
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
    // `onError` is a sink, not a reason to reload.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, tick]);
  return { jobs, refresh: () => setTick((t) => t + 1) };
}

/** Queued and running exports, for the background-tasks button. */
export function useActiveExportCount(): number {
  const { jobs } = useExportJobs();
  return (jobs ?? []).filter(jobActive).length;
}

export const fmtSeconds = (s: number) =>
  s >= 60
    ? `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`
    : `${s.toFixed(2).replace(/\.?0+$/, "")} s`;

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

const QUALITIES = ["very low", "low", "medium", "high", "very high"];
const SIZES = [
  { id: "match", label: "Timeline size" },
  { id: "720", label: "720p" },
  { id: "1080", label: "1080p" },
  { id: "1440", label: "1440p" },
  { id: "2160", label: "4K" },
];
const RATES = ["match", "24", "25", "30", "50", "60"];
const WAV_FORM: ExportForm = {
  container: "wav",
  video: null,
  audio: { codec: "pcm", bitDepth: 24, bitrate: null },
  size: "match",
  fps: "match",
  range: null,
  loudness: null,
};

/** Fits a form to a newly chosen container: codecs it cannot hold become its first allowed ones. */
function fitContainer(
  form: ExportForm,
  containerId: string,
  options: ExportOptions,
): ExportForm {
  const c = options.containers.find((x) => x.id === containerId);
  if (!c) return { ...form, container: containerId };
  let video = form.video;
  if (c.audioOnly) video = null;
  else if (!video || !c.videoCodecs.includes(video.codec))
    video = {
      codec: c.videoCodecs[0],
      profile: null,
      bitDepth: null,
      quality: video?.quality ?? "high",
      alpha: false,
    };
  let audio = form.audio;
  if (c.audioOnly && !audio) audio = { codec: c.audioCodecs[0], bitDepth: null, bitrate: null };
  if (audio && !c.audioCodecs.includes(audio.codec))
    audio = { codec: c.audioCodecs[0], bitDepth: null, bitrate: null };
  return { ...form, container: containerId, video, audio };
}

/** The path with the container's extension. */
const withExtension = (path: string, ext: string) =>
  /\.[a-z0-9]{2,4}$/i.test(path) ? path.replace(/\.[a-z0-9]{2,4}$/i, `.${ext}`) : `${path}.${ext}`;

type Phase = { kind: "setup" } | { kind: "queued"; id: string };

/** One select of the Advanced section. */
function Pick({
  label,
  value,
  onChange,
  choices,
  field,
  bad,
  testId,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  choices: { id: string; label: string; available?: boolean }[];
  field: string;
  bad: string[];
  testId: string;
}) {
  return (
    <label className={`xp-pick${bad.includes(field) ? " xp-invalid" : ""}`}>
      <span>{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        data-testid={testId}
        aria-invalid={bad.includes(field)}
      >
        {choices.map((c) => (
          <option key={c.id} value={c.id}>
            {c.label}
            {c.available === false ? " (not on this computer)" : ""}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Progress, result and cancel of the job the dialog queued. */
function QueuedJob({
  id,
  onClose,
  onAnother,
  onError,
}: {
  id: string;
  onClose: () => void;
  onAnother: () => void;
  onError: (e: AppError) => void;
}) {
  const { jobs, refresh } = useExportJobs(onError);
  const job = jobs?.find((j) => j.id === id);
  if (!job) return <p className="muted small">Queueing…</p>;
  const ahead = (jobs ?? []).filter(
    (j) => j.status === "running" || (j.status === "queued" && (j.createdAt ?? 0) < (job.createdAt ?? 0)),
  ).length;
  const showAll = () => {
    openExportsPanel();
    onClose();
  };
  if (job.status === "done")
    return (
      <div className="xp-result" data-testid="export-done">
        <p>
          Saved <strong>{job.fileName}</strong>
          {job.secondsTaken != null && (
            <span className="muted"> in {job.secondsTaken.toFixed(1)} s</span>
          )}
          {job.encoder && <span className="muted"> · {job.encoder}</span>}
        </p>
        {job.loudness && <p className="muted small">{job.loudness}</p>}
        {job.warnings.map((w) => (
          <p key={w} className="muted small" data-testid="export-warning">
            {w}
          </p>
        ))}
        <div className="xp-actions">
          <button
            className="btn"
            onClick={() => runCommand(commands.revealPath(job.out), undefined, onError)}
            data-testid="export-reveal"
          >
            Show in folder
          </button>
          <button className="btn" onClick={onAnother}>
            Export another
          </button>
          <button className="btn primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    );
  if (job.status === "failed" || job.status === "cancelled")
    return (
      <div className="xp-result" data-testid="export-error">
        <p className={job.status === "failed" ? "xp-error" : ""}>
          {job.status === "cancelled" ? "The export was cancelled." : job.error}
        </p>
        <div className="xp-actions">
          <button className="btn" onClick={onAnother}>
            Back
          </button>
          <button className="btn primary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    );
  return (
    <>
      <p className="small">
        <strong>{job.fileName}</strong> <span className="muted">· {job.summary}</span>
      </p>
      <div className="xp-progress" data-testid="export-progress" data-status={job.status}>
        <div
          className="xp-progress-bar"
          style={{ width: `${((job.progress ?? 0) * 100).toFixed(1)}%` }}
        />
        <span className="small">
          {job.status === "queued" ? (
            <span className="muted">
              Waiting for {ahead} earlier export{ahead === 1 ? "" : "s"}
            </span>
          ) : (
            <>
              {Math.round((job.progress ?? 0) * 100)}%{" "}
              {job.totalFrames > 0 && (
                <span className="muted">
                  frame {job.frame} / {job.totalFrames}
                </span>
              )}
            </>
          )}
        </span>
      </div>
      <p className="muted small">
        The export goes on if you close this window or keep editing; it renders the project as it was when you
        clicked Export.
      </p>
      <div className="xp-actions">
        <button
          className="btn"
          onClick={() => runCommand(commands.cancelExportJob(job.id), () => refresh(), onError)}
          data-testid="export-cancel"
        >
          Cancel export
        </button>
        <button className="btn" onClick={showAll} data-testid="export-show-exports">
          All exports
        </button>
        <button className="btn primary" onClick={onClose}>
          Close
        </button>
      </div>
    </>
  );
}

/** Export dialog: presets, Advanced settings, range, audio only, loudness and destination; the job goes into the
 * export queue, and its progress shows here and in the Exports panel. */
export function ExportDialog({
  projectName,
  range: marked,
  onClose,
  onError,
}: {
  projectName: string;
  /** The timeline's aspect ratio (the summary under the settings gives the exact frame size). */
  aspect?: string;
  /** A marked range (seconds) to offer, and to start with when the dialog was opened for it. */
  range?: [number, number] | null;
  onClose: () => void;
  onError: (e: AppError) => void;
}) {
  const [options, setOptions] = useState<ExportOptions | null>(null);
  const [preset, setPreset] = useState<string | null>(null);
  const [form, setForm] = useState<ExportForm | null>(null);
  /** The video settings to return to when "Audio only" is switched off. */
  const [lastVideo, setLastVideo] = useState<ExportForm | null>(null);
  const [useRange, setUseRange] = useState(!!marked);
  const [span, setSpan] = useState<[number, number]>(marked ?? [0, 0]);
  const [check, setCheck] = useState<ExportCheck | null>(null);
  /** The chosen file; its extension follows the container. */
  const [chosen, setPath] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "setup" });
  const [what, setWhat] = useState<"video" | "timeline">("video");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    fire(
      commands.exportOptions().then((r) => {
        if (!live) return;
        if (r.status === "error") return onError(r.error);
        setOptions(r.data);
        const first =
          r.data.presets.find((p) => !p.unavailable && p.form.video) ?? r.data.presets[0];
        if (first) {
          setPreset(first.name);
          setForm(first.form);
        }
      }),
      onError,
    );
    return () => {
      live = false;
    };
    // Loaded once per dialog.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** The form as it is checked and queued: with the chosen range. */
  const effective: ExportForm | null = form
    ? { ...form, range: useRange ? [span[0], span[1]] : null }
    : null;
  const key = JSON.stringify(effective);
  useEffect(() => {
    if (!effective) return;
    let live = true;
    fire(
      commands.checkExport(effective).then((r) => {
        if (!live) return;
        if (r.status === "error") onError(r.error);
        else setCheck(r.data);
      }),
      onError,
    );
    return () => {
      live = false;
    };
    // Checked again whenever the settings change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const ext = check?.extension ?? form?.container ?? "mp4";
  const path = chosen ? withExtension(chosen, ext) : null;

  const edit = (f: ExportForm) => {
    setForm(f);
    setPreset(null);
  };
  const choosePreset = (p: ExportPresetView) => {
    setPreset(p.name);
    setForm(p.form);
  };
  const audioOnly = !!form && form.video === null;
  const toggleAudioOnly = () => {
    if (!form) return;
    if (audioOnly) {
      setForm(lastVideo ?? options?.presets.find((p) => p.form.video)?.form ?? form);
      setPreset(null);
    } else {
      setLastVideo(form);
      edit({ ...WAV_FORM, loudness: form.loudness });
    }
  };

  const choose = async () => {
    const r = await commands.chooseExportPath(projectName || "Export", ext);
    if (r.status === "error") onError(r.error);
    else if (r.data) setPath(r.data);
  };
  const start = async () => {
    if (!effective) return;
    let dest = path;
    if (!dest) {
      const r = await commands.chooseExportPath(projectName || "Export", ext);
      if (r.status === "error") return onError(r.error);
      if (!r.data) return;
      dest = r.data;
      setPath(dest);
    }
    setBusy(true);
    try {
      const r = await commands.queueExport(effective, dest);
      if (r.status === "error") onError(r.error);
      else setPhase({ kind: "queued", id: r.data.id });
    } finally {
      setBusy(false);
    }
  };

  const bad = check && !check.ok ? check.fields : [];
  const container = options?.containers.find((c) => c.id === form?.container);
  const codec = options?.videoCodecs.find((c) => c.id === form?.video?.codec);
  const setVideo = (patch: Partial<NonNullable<ExportForm["video"]>>) =>
    form?.video && edit({ ...form, video: { ...form.video, ...patch } });
  const setAudio = (patch: Partial<NonNullable<ExportForm["audio"]>> | null) =>
    form &&
    edit({
      ...form,
      audio:
        patch === null
          ? null
          : { ...(form.audio ?? { codec: container?.audioCodecs[0] ?? "aac", bitDepth: null, bitrate: null }), ...patch },
    });
  const presetGroups = options
    ? [
        { title: "Video", list: options.presets.filter((p) => p.form.video) },
        { title: "Sound only", list: options.presets.filter((p) => !p.form.video) },
      ]
    : [];

  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="Export"
      data-testid="export-dialog"
    >
      <div className="modal xp-modal xp-wide">
        <header>
          <h3>Export</h3>
          <button className="icon-btn" aria-label="Close" onClick={onClose}>
            <Icon name="close" size={16} />
          </button>
        </header>
        {phase.kind === "setup" && (
          <div className="xp-seg xp-kind" role="tablist" data-testid="export-kind">
            <button
              role="tab"
              aria-selected={what === "video"}
              className={what === "video" ? "on" : ""}
              onClick={() => setWhat("video")}
            >
              Media file
            </button>
            <button
              role="tab"
              aria-selected={what === "timeline"}
              className={what === "timeline" ? "on" : ""}
              onClick={() => setWhat("timeline")}
              data-testid="export-kind-timeline"
            >
              Timeline for another editor
            </button>
          </div>
        )}
        {phase.kind === "queued" ? (
          <QueuedJob
            id={phase.id}
            onClose={onClose}
            onAnother={() => setPhase({ kind: "setup" })}
            onError={onError}
          />
        ) : what === "timeline" ? (
          <TimelineExport onClose={onClose} onError={onError} />
        ) : !options || !form ? (
          <p className="muted small">Loading formats…</p>
        ) : (
          <>
            <div className="xp-presets" data-testid="export-presets">
              {presetGroups.map((g) => (
                <div key={g.title} className="xp-preset-group">
                  <span className="xp-group-title">{g.title}</span>
                  <div className="xp-preset-list" role="radiogroup" aria-label={`${g.title} presets`}>
                    {g.list.map((p) => (
                      <button
                        key={p.name}
                        role="radio"
                        aria-checked={preset === p.name}
                        className={`xp-preset${preset === p.name ? " on" : ""}`}
                        disabled={!!p.unavailable}
                        title={p.unavailable ?? p.about}
                        onClick={() => choosePreset(p)}
                        data-testid={`export-preset-${slug(p.name)}`}
                      >
                        <strong>{p.name}</strong>
                        <span className="muted">
                          {p.unavailable ? "Not on this computer" : p.summary}
                        </span>
                      </button>
                    ))}
                  </div>
                </div>
              ))}
            </div>

            <div className="xp-row">
              <label className="xp-check">
                <input
                  type="checkbox"
                  checked={audioOnly}
                  onChange={toggleAudioOnly}
                  data-testid="export-audio-only"
                />
                Sound only
              </label>
              <label className={`xp-pick xp-inline${bad.includes("loudness") ? " xp-invalid" : ""}`}>
                <span>Loudness</span>
                <select
                  value={form.loudness == null ? "off" : String(form.loudness)}
                  onChange={(e) =>
                    edit({ ...form, loudness: e.target.value === "off" ? null : Number(e.target.value) })
                  }
                  data-testid="export-loudness"
                >
                  <option value="off">As mixed</option>
                  {options.loudnessTargets.map((l) => (
                    <option key={l} value={String(l)}>
                      {l} LUFS
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <div className={`xp-field${bad.includes("range") ? " xp-invalid" : ""}`}>
              <span>Range</span>
              <div className="xp-row">
                <div className="xp-seg" role="radiogroup" aria-label="Range">
                  <button
                    role="radio"
                    aria-checked={!useRange}
                    className={!useRange ? "on" : ""}
                    onClick={() => setUseRange(false)}
                    data-testid="export-range-all"
                  >
                    Whole timeline
                  </button>
                  <button
                    role="radio"
                    aria-checked={useRange}
                    className={useRange ? "on" : ""}
                    onClick={() => setUseRange(true)}
                    data-testid="export-range-part"
                  >
                    {marked ? "Marked range" : "Part"}
                  </button>
                </div>
                {useRange && (
                  <span className="xp-range">
                    <input
                      type="number"
                      min={0}
                      step={0.1}
                      value={span[0]}
                      aria-label="Range start (seconds)"
                      onChange={(e) => setSpan([Number(e.target.value), span[1]])}
                      data-testid="export-range-start"
                    />
                    –
                    <input
                      type="number"
                      min={0}
                      step={0.1}
                      value={span[1]}
                      aria-label="Range end (seconds)"
                      onChange={(e) => setSpan([span[0], Number(e.target.value)])}
                      data-testid="export-range-end"
                    />
                    <span className="muted small">s</span>
                  </span>
                )}
              </div>
            </div>

            <details className="xp-advanced" data-testid="export-advanced">
              <summary>Advanced</summary>
              <div className="xp-grid">
                <Pick
                  label="Container"
                  value={form.container}
                  onChange={(v) => edit(fitContainer(form, v, options))}
                  choices={options.containers.filter((c) => c.audioOnly === audioOnly)}
                  field="container"
                  bad={bad}
                  testId="export-container"
                />
                {form.video && container && (
                  <>
                    <Pick
                      label="Video codec"
                      value={form.video.codec}
                      onChange={(v) => setVideo({ codec: v, profile: null, bitDepth: null, alpha: false })}
                      choices={container.videoCodecs.map(
                        (id) => options.videoCodecs.find((c) => c.id === id) ?? { id, label: id },
                      )}
                      field="video.codec"
                      bad={bad}
                      testId="export-video-codec"
                    />
                    {codec && codec.profiles.length > 0 && (
                      <Pick
                        label="Profile"
                        value={form.video.profile ?? ""}
                        onChange={(v) => setVideo({ profile: v || null })}
                        choices={[{ id: "", label: "Default" }, ...codec.profiles]}
                        field="video.profile"
                        bad={bad}
                        testId="export-profile"
                      />
                    )}
                    <Pick
                      label="Bit depth"
                      value={form.video.bitDepth == null ? "" : String(form.video.bitDepth)}
                      onChange={(v) => setVideo({ bitDepth: v ? Number(v) : null })}
                      choices={[
                        { id: "", label: "Default" },
                        { id: "8", label: "8-bit" },
                        { id: "10", label: "10-bit" },
                        { id: "12", label: "12-bit" },
                      ]}
                      field="video.bitDepth"
                      bad={bad}
                      testId="export-bit-depth"
                    />
                    <Pick
                      label="Quality"
                      value={form.video.quality}
                      onChange={(v) => setVideo({ quality: v })}
                      choices={QUALITIES.map((q) => ({ id: q, label: q[0].toUpperCase() + q.slice(1) }))}
                      field="video.quality"
                      bad={bad}
                      testId="export-quality"
                    />
                    <Pick
                      label="Size"
                      value={form.size}
                      onChange={(v) => edit({ ...form, size: v })}
                      choices={SIZES}
                      field="size"
                      bad={bad}
                      testId="export-size"
                    />
                    <Pick
                      label="Frame rate"
                      value={form.fps}
                      onChange={(v) => edit({ ...form, fps: v })}
                      choices={RATES.map((r) => ({ id: r, label: r === "match" ? "Timeline rate" : `${r} fps` }))}
                      field="fps"
                      bad={bad}
                      testId="export-fps"
                    />
                    <label className={`xp-check${bad.includes("video.alpha") ? " xp-invalid" : ""}`}>
                      <input
                        type="checkbox"
                        checked={form.video.alpha}
                        onChange={(e) => setVideo({ alpha: e.target.checked })}
                        data-testid="export-alpha"
                      />
                      Transparent background
                    </label>
                  </>
                )}
                {container && (
                  <Pick
                    label="Audio codec"
                    value={form.audio?.codec ?? ""}
                    onChange={(v) => setAudio(v ? { codec: v, bitDepth: null, bitrate: null } : null)}
                    choices={[
                      ...(audioOnly ? [] : [{ id: "", label: "No sound" }]),
                      ...container.audioCodecs.map(
                        (id) => options.audioCodecs.find((c) => c.id === id) ?? { id, label: id },
                      ),
                    ]}
                    field="audio.codec"
                    bad={bad}
                    testId="export-audio-codec"
                  />
                )}
                {form.audio && (form.audio.codec === "pcm" || form.audio.codec === "flac") && (
                  <Pick
                    label="Sample depth"
                    value={String(form.audio.bitDepth ?? 24)}
                    onChange={(v) => setAudio({ bitDepth: Number(v) })}
                    choices={[
                      { id: "16", label: "16-bit" },
                      { id: "24", label: "24-bit" },
                    ]}
                    field="audio.bitDepth"
                    bad={bad}
                    testId="export-audio-bit-depth"
                  />
                )}
              </div>
            </details>

            <div className="xp-field">
              <span>Save to</span>
              <div className="xp-dest">
                <span className="xp-dest-path" title={path ?? ""}>
                  {path ?? "Choose a location…"}
                </span>
                <button
                  className="btn"
                  onClick={() => fire(choose(), onError)}
                  data-testid="export-choose"
                >
                  Choose…
                </button>
              </div>
            </div>
            {check && !check.ok ? (
              <p className="xp-error small" role="alert" data-testid="export-invalid">
                {check.message}
              </p>
            ) : (
              <p className="muted small" data-testid="export-summary">
                {check?.summary ?? ""}
              </p>
            )}
            <div className="xp-actions">
              <button className="btn" onClick={onClose}>
                Close
              </button>
              <button
                className="btn primary"
                onClick={() => fire(start(), onError)}
                disabled={busy || !check?.ok}
                data-testid="export-start"
              >
                Export
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** "Save range as media" and "Render in place": renders the range into a new media file of the project. Render in
 * place asks first, then replaces what the range covers on the video tracks with one clip, as one undo step. */
export function RangeMediaDialog({
  start,
  end,
  replace,
  onClose,
  onError,
}: {
  start: number;
  end: number;
  replace: boolean;
  onClose: () => void;
  onError: (e: AppError) => void;
}) {
  const [state, setState] = useState<"confirm" | "busy" | SavedRange>(replace ? "confirm" : "busy");
  const what = `${fmtSeconds(start)}–${fmtSeconds(end)}`;
  const render = () =>
    commands.saveRangeAsMedia(start, end, replace).then((r) => {
      if (r.status === "error") {
        onError(r.error);
        onClose();
      } else setState(r.data);
    });
  const run = () => {
    setState("busy");
    fire(render(), onError);
  };
  useEffect(() => {
    if (!replace) fire(render(), onError);
    // Runs once when opened for "Save range as media".
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const title = replace ? "Render in place" : "Save range as media";
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true" aria-label={title} data-testid="range-media-dialog">
      <div className="modal xp-modal">
        <header>
          <h3>{title}</h3>
          <button className="icon-btn" aria-label="Close" onClick={onClose} disabled={state === "busy"}>
            <Icon name="close" size={16} />
          </button>
        </header>
        {state === "confirm" ? (
          <>
            <p className="small">
              Render {what} into a new clip and put it in place of everything the range covers on the video tracks?
              One undo brings the original items back; the rendered file stays in your media.
            </p>
            <div className="xp-actions">
              <button className="btn" onClick={onClose}>
                Cancel
              </button>
              <button className="btn primary" onClick={run} data-testid="range-confirm">
                Render in place
              </button>
            </div>
          </>
        ) : state === "busy" ? (
          <p className="small" data-testid="range-busy">
            Rendering {what}…
          </p>
        ) : (
          <div className="xp-result" data-testid="range-done">
            <p>
              {state.replaced ? "Rendered in place: " : "Added to your media: "}
              <strong>{state.file.split("/").pop()}</strong>
              {state.format && <span className="muted"> · {state.format}</span>}
            </p>
            <div className="xp-actions">
              <button className="btn primary" onClick={onClose}>
                Done
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
