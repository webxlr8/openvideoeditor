// Home's "Changelog" (what's new in this release) and "Send feedback" (a report the tester copies; nothing is sent
// from the app).

import { useEffect, useState, type ReactNode } from "react";
import type { AppError, CrashReport, FeedbackInfo_Serialize } from "./bindings";
import { commands } from "./bridge";
import { copyText } from "./clipboard";
import { Icon } from "./icons";
import { fire, logError, runCommand } from "./notify";
import { RELEASE_NOTES } from "./releaseNotes";
import "./support.css";

/** Settings → Privacy: whether the person agreed to put crash reports in feedback. */
export const CRASH_OPT_IN_KEY = "ove.crashReports";

export function crashReportsOptedIn(): boolean {
  try {
    return localStorage.getItem(CRASH_OPT_IN_KEY) === "on";
  } catch {
    return false;
  }
}

/** The text "Copy report" puts on the clipboard. */
export function buildReport(
  description: string,
  environment: string,
  crash?: { at: number | null; text: string } | null,
): string {
  const parts = [
    description.trim() || "(no description)",
    `--- Environment ---\n${environment}`,
  ];
  if (crash)
    parts.push(
      `--- Latest crash report${crash.at ? ` (${new Date(crash.at).toISOString()})` : ""} ---\n${crash.text.trim()}`,
    );
  return parts.join("\n\n") + "\n";
}

function Modal({
  label,
  testId,
  className,
  onClose,
  children,
}: {
  label: string;
  testId: string;
  className: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onMouseDown={onClose} data-testid={testId}>
      <div
        className={`modal ${className}`}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header>
          <h3>{label}</h3>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <Icon name="close" size={16} />
          </button>
        </header>
        {children}
      </div>
    </div>
  );
}

export function WhatsNew({ onClose }: { onClose: () => void }) {
  const n = RELEASE_NOTES;
  return (
    <Modal
      label="What's new"
      testId="whats-new"
      className="support-modal"
      onClose={onClose}
    >
      <p className="support-release">
        <strong>{n.name}</strong> <span className="muted small">{n.tag}</span>
      </p>
      <p className="muted">{n.intro}</p>
      <h4>What you can do</h4>
      <ul className="support-list" data-testid="whats-new-highlights">
        {n.highlights.map((h) => (
          <li key={h.title}>
            <strong>{h.title}.</strong> {h.text}
          </li>
        ))}
      </ul>
      <h4>Privacy</h4>
      <ul className="support-list">
        {n.privacy.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
      <h4>Known limitations</h4>
      <ul className="support-list" data-testid="whats-new-limitations">
        {n.limitations.map((h) => (
          <li key={h.title}>
            <strong>{h.title}.</strong> {h.text}
          </li>
        ))}
      </ul>
      <p className="muted small">
        Found a problem? Use Send feedback on Home to put together a report.
      </p>
    </Modal>
  );
}

export function FeedbackDialog({
  onClose,
  onError,
}: {
  onClose: () => void;
  onError: (e: AppError) => void;
}) {
  const [text, setText] = useState("");
  const [info, setInfo] = useState<FeedbackInfo_Serialize | null>(null);
  const [optedIn] = useState(crashReportsOptedIn);
  const [latest, setLatest] = useState<CrashReport | null>(null);
  const [crashText, setCrashText] = useState<string | null>(null);
  const [attach, setAttach] = useState(true);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    runCommand(commands.feedbackInfo(), setInfo, onError);
    fire(
      commands.crashReports().then((reports) => {
        const newest = reports[0] ?? null;
        setLatest(newest);
        // The report's text is read only when the person agreed to share crash reports.
        if (newest && optedIn)
          runCommand(
            commands.readCrashReport(newest.path),
            setCrashText,
            logError,
          );
      }),
      logError,
    );
  }, [onError, optedIn]);

  const environment = info?.environment ?? "…";
  const crash =
    optedIn && attach && latest && crashText !== null
      ? { at: latest.at, text: crashText }
      : null;
  const report = buildReport(text, environment, crash);

  const copy = () =>
    copyText(report)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch((e: unknown) =>
        onError({
          code: "internal",
          message: `Couldn't copy the report (${e instanceof Error ? e.message : String(e)}). Open "What will be copied", select the text and copy it yourself.`,
        }),
      );

  return (
    <Modal
      label="Send feedback"
      testId="feedback-dialog"
      className="support-modal"
      onClose={onClose}
    >
      <p className="muted">
        Tell us what you did, what you expected and what happened. Your Mac or
        PC model helps too.
      </p>
      <textarea
        className="support-text"
        rows={6}
        autoFocus
        value={text}
        placeholder="What happened?"
        aria-label="Your feedback"
        data-testid="feedback-text"
        onChange={(e) => setText(e.target.value)}
      />
      <h4>Added to your report</h4>
      <pre className="support-env" data-testid="feedback-env">
        {environment}
      </pre>
      {latest &&
        (optedIn ? (
          <label className="gen-check">
            <input
              type="checkbox"
              checked={attach}
              disabled={crashText === null}
              onChange={(e) => setAttach(e.target.checked)}
              data-testid="feedback-attach-crash"
            />{" "}
            Include the latest crash report
            {latest.at ? ` (${new Date(latest.at).toLocaleString()})` : ""}
          </label>
        ) : (
          <p className="muted small" data-testid="feedback-crash-hint">
            A crash report is saved on this computer. To include it, turn on
            crash reports in Settings → Privacy.
          </p>
        ))}
      <details className="support-preview">
        <summary>What will be copied</summary>
        <pre data-testid="feedback-preview">{report}</pre>
      </details>
      <p className="muted small">
        Nothing is sent from the app. Copy the report and send it in reply to
        your invitation
        {info?.feedbackUrl ? " or on the feedback page" : ""}.
      </p>
      <div className="xp-actions">
        {info?.feedbackUrl && (
          <button
            className="btn"
            data-testid="feedback-open-page"
            onClick={() =>
              runCommand(commands.openUrl(info.feedbackUrl!), undefined, onError)
            }
          >
            Open feedback page
          </button>
        )}
        <button className="btn" onClick={onClose}>
          Close
        </button>
        <button
          className="btn primary"
          data-testid="feedback-copy"
          disabled={!info}
          onClick={() => fire(copy(), onError)}
        >
          <Icon name="copy" size={14} /> {copied ? "Copied" : "Copy report"}
        </button>
      </div>
    </Modal>
  );
}
