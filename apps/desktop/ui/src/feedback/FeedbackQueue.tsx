// Help → Feedback (unify-agent-tool-layer, D8): reports agents drafted with `send_feedback`, kept on this computer.
// The person reads each one, edits and saves it, copies it to send it themselves, or deletes it. Nothing here sends
// anything anywhere.

import { useEffect, useState } from "react";
import type { AppError, FeedbackDraft_Serialize as Draft } from "../bindings";
import { commands } from "../bridge";
import { copyText } from "../clipboard";
import { Icon } from "../icons";
import { fire, runCommand } from "../notify";
import { Modal } from "../Support";
import "./feedback.css";

export function FeedbackQueue({
  onClose,
  onError,
}: {
  onClose: () => void;
  onError: (e: AppError) => void;
}) {
  const [drafts, setDrafts] = useState<Draft[] | null>(null);
  useEffect(() => {
    runCommand(commands.feedbackDrafts(), setDrafts, onError);
  }, [onError]);
  const replace = (d: Draft) =>
    setDrafts((ds) => (ds ?? []).map((x) => (x.id === d.id ? d : x)));
  const remove = (id: string) =>
    runCommand(
      commands.deleteFeedbackDraft(id),
      () => setDrafts((ds) => (ds ?? []).filter((x) => x.id !== id)),
      onError,
    );
  return (
    <Modal
      label="Feedback"
      testId="feedback-queue"
      className="support-modal"
      onClose={onClose}
    >
      <p className="muted">
        Reports that agents drafted about problems with the app. They are kept
        on this computer and never sent: read each one, change what you like,
        and copy it to send it yourself, or delete it.
      </p>
      {drafts === null ? (
        <p className="muted">Loading…</p>
      ) : drafts.length === 0 ? (
        <p className="muted" data-testid="feedback-queue-empty">
          No reports. When an agent you work with runs into a problem with the
          app, it can draft one here for you to review.
        </p>
      ) : (
        <ul className="feedback-drafts" data-testid="feedback-drafts">
          {drafts.map((d) => (
            <DraftRow
              key={d.id}
              draft={d}
              onSaved={replace}
              onDelete={() => remove(d.id)}
              onError={onError}
            />
          ))}
        </ul>
      )}
      <div className="xp-actions">
        <button className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </Modal>
  );
}

function DraftRow({
  draft,
  onSaved,
  onDelete,
  onError,
}: {
  draft: Draft;
  onSaved: (d: Draft) => void;
  onDelete: () => void;
  onError: (e: AppError) => void;
}) {
  const [text, setText] = useState(draft.summary);
  const [copied, setCopied] = useState(false);
  const changed = text.trim() !== draft.summary;
  const save = () =>
    runCommand(commands.saveFeedbackDraft(draft.id, text), onSaved, onError);
  const copy = () =>
    copyText(draft.report)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch((e: unknown) =>
        onError({
          code: "internal",
          message: `Couldn't copy the report (${e instanceof Error ? e.message : String(e)}).`,
        }),
      );
  return (
    <li className="feedback-draft" data-testid="feedback-draft" data-id={draft.id}>
      <div className="feedback-draft-head">
        <span className={`feedback-tag sev-${draft.severity}`}>
          {draft.category} · {draft.severity}
        </span>
        <span className="muted small">
          from {draft.from}
          {draft.createdAt ? ` · ${new Date(draft.createdAt).toLocaleString()}` : ""}
          {draft.edited ? " · edited" : ""}
        </span>
      </div>
      <textarea
        className="support-text"
        rows={3}
        value={text}
        aria-label="Report"
        data-testid="feedback-draft-text"
        onChange={(e) => setText(e.target.value)}
      />
      {(draft.recentTools.length > 0 || draft.lastError) && (
        <p className="muted small">
          {draft.recentTools.length > 0 && `Recent tools: ${draft.recentTools.join(", ")}. `}
          {draft.lastError && `Last error: ${draft.lastError}.`}
        </p>
      )}
      <div className="feedback-draft-actions">
        <button
          className="btn"
          data-testid="feedback-draft-delete"
          onClick={onDelete}
        >
          Delete
        </button>
        <button
          className="btn"
          data-testid="feedback-draft-save"
          disabled={!changed}
          onClick={save}
        >
          Save
        </button>
        <button
          className="btn primary"
          data-testid="feedback-draft-copy"
          disabled={changed}
          title={changed ? "Save your changes first" : undefined}
          onClick={() => fire(copy(), onError)}
        >
          <Icon name="copy" size={14} /> {copied ? "Copied" : "Copy report"}
        </button>
      </div>
    </li>
  );
}
