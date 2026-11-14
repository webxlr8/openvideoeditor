import { useEffect, useState } from "react";
import type {
  AppError,
  ExportProgress_Serialize,
  TimelineExportReport,
} from "./bindings";
import { commands, events } from "./bridge";
import { Icon } from "./icons";
import { fire, runCommand, unlisten } from "./notify";
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

const RESOLUTIONS = [
  { label: "720p", height: 720 },
  { label: "1080p", height: 1080 },
  { label: "4K", height: 2160 },
];
const RATES = [24, 25, 30, 60];

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

function size(aspect: string, height: number) {
  const [a, b] = aspect.split(":").map(Number);
  const w = Math.round((height * (a || 16)) / (b || 9) / 2) * 2;
  return `${w}×${height}`;
}

type Phase =
  | { kind: "setup" }
  | { kind: "running"; p: ExportProgress_Serialize | null }
  | { kind: "done"; p: ExportProgress_Serialize };

/** Export dialog: resolution, frame rate, destination, progress, cancel, reveal. */
export function ExportDialog({
  projectName,
  aspect,
  onClose,
  onError,
}: {
  projectName: string;
  aspect: string;
  onClose: () => void;
  onError: (e: AppError) => void;
}) {
  const [height, setHeight] = useState(1080);
  const [fps, setFps] = useState(30);
  const [path, setPath] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "setup" });
  const [what, setWhat] = useState<"video" | "timeline">("video");

  useEffect(() => {
    const un = events.exportProgress.listen((e) => {
      const p = e.payload;
      setPhase(p.done ? { kind: "done", p } : { kind: "running", p });
    });
    return () => unlisten(un);
  }, []);

  const choose = async () => {
    const r = await commands.chooseExportPath(projectName || "Export");
    if (r.status === "error") onError(r.error);
    else if (r.data) setPath(r.data);
  };
  const start = async () => {
    let dest = path;
    if (!dest) {
      const r = await commands.chooseExportPath(projectName || "Export");
      if (r.status === "error") return onError(r.error);
      if (!r.data) return;
      dest = r.data;
      setPath(dest);
    }
    setPhase({ kind: "running", p: null });
    const r = await commands.exportProject({ path: dest, height, fps });
    if (r.status === "error") {
      onError(r.error);
      setPhase({ kind: "setup" });
    }
  };

  const running = phase.kind === "running";
  const fraction =
    phase.kind === "running"
      ? (phase.p?.fraction ?? 0)
      : phase.kind === "done"
        ? 1
        : 0;
  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="Export"
      data-testid="export-dialog"
    >
      <div className="modal xp-modal">
        <header>
          <h3>Export</h3>
          <button
            className="icon-btn"
            aria-label="Close"
            onClick={onClose}
            disabled={running}
          >
            <Icon name="close" size={16} />
          </button>
        </header>
        {phase.kind === "setup" && (
          <div
            className="xp-seg xp-kind"
            role="tablist"
            data-testid="export-kind"
          >
            <button
              role="tab"
              aria-selected={what === "video"}
              className={what === "video" ? "on" : ""}
              onClick={() => setWhat("video")}
            >
              Video file
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
        {what === "timeline" && phase.kind === "setup" ? (
          <TimelineExport onClose={onClose} onError={onError} />
        ) : phase.kind === "done" ? (
          phase.p.error ? (
            <div className="xp-result" data-testid="export-error">
              <p className="xp-error">{phase.p.error}</p>
              <div className="xp-actions">
                <button
                  className="btn"
                  onClick={() => setPhase({ kind: "setup" })}
                >
                  Back
                </button>
              </div>
            </div>
          ) : (
            <div className="xp-result" data-testid="export-done">
              <p>
                Saved <strong>{phase.p.path?.split(/[\\/]/).pop()}</strong>
                {phase.p.secondsTaken != null && (
                  <span className="muted">
                    {" "}
                    in {phase.p.secondsTaken.toFixed(1)} s
                  </span>
                )}
                {phase.p.encoder && (
                  <span className="muted"> · {phase.p.encoder}</span>
                )}
              </p>
              {phase.p.warning && <p className="muted" data-testid="export-warning">{phase.p.warning}</p>}
              <div className="xp-actions">
                <button
                  className="btn"
                  onClick={() => {
                    if (phase.p.path)
                      runCommand(
                        commands.revealPath(phase.p.path),
                        undefined,
                        onError,
                      );
                  }}
                  data-testid="export-reveal"
                >
                  Show in folder
                </button>
                <button className="btn primary" onClick={onClose}>
                  Done
                </button>
              </div>
            </div>
          )
        ) : (
          <>
            <div className="xp-field">
              <span>Resolution</span>
              <div className="xp-seg">
                {RESOLUTIONS.map((r) => (
                  <button
                    key={r.height}
                    className={height === r.height ? "on" : ""}
                    disabled={running}
                    onClick={() => setHeight(r.height)}
                  >
                    {r.label}
                  </button>
                ))}
              </div>
              <span className="muted small">
                {size(aspect, height)} · {aspect}
              </span>
            </div>
            <div className="xp-field">
              <span>Frame rate</span>
              <div className="xp-seg">
                {RATES.map((r) => (
                  <button
                    key={r}
                    className={fps === r ? "on" : ""}
                    disabled={running}
                    onClick={() => setFps(r)}
                  >
                    {r}
                  </button>
                ))}
              </div>
            </div>
            <div className="xp-field">
              <span>Save to</span>
              <div className="xp-dest">
                <span className="xp-dest-path" title={path ?? ""}>
                  {path ?? "Choose a location…"}
                </span>
                <button
                  className="btn"
                  onClick={() => fire(choose(), onError)}
                  disabled={running}
                  data-testid="export-choose"
                >
                  Choose…
                </button>
              </div>
            </div>
            <p className="muted small">MP4 · H.264 video · AAC audio</p>
            {running && (
              <div className="xp-progress" data-testid="export-progress">
                <div
                  className="xp-progress-bar"
                  style={{ width: `${(fraction * 100).toFixed(1)}%` }}
                />
                <span className="small">
                  {Math.round(fraction * 100)}%{" "}
                  {phase.p && (
                    <span className="muted">
                      frame {phase.p.frame} / {phase.p.totalFrames}
                    </span>
                  )}
                </span>
              </div>
            )}
            <div className="xp-actions">
              {running ? (
                <button
                  className="btn"
                  onClick={() =>
                    runCommand(commands.cancelExport(), undefined, onError)
                  }
                  data-testid="export-cancel"
                >
                  Cancel
                </button>
              ) : (
                <>
                  <button className="btn" onClick={onClose}>
                    Close
                  </button>
                  <button
                    className="btn primary"
                    onClick={() => fire(start(), onError)}
                    data-testid="export-start"
                  >
                    Export
                  </button>
                </>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
