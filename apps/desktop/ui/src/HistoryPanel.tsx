// The project's history: every revision, newest first, with who made it (you, the assistant, the command line, an
// MCP agent, a template or a generation) and when. "Restore to before this" puts the project back as it was before
// a change, as one new revision, so it can itself be undone.

import { useEffect, useState } from "react";
import type { AppError, HistoryRow } from "./bindings";
import { commands } from "./bridge";
import type { EditResult } from "./edit";
import { Icon } from "./icons";
import { runCommand } from "./notify";
import { formatWhen } from "./timecode";

/** Revision summaries the assistant writes start with this; the "who" column already says so. */
function shortSummary(row: HistoryRow): string {
  const s = row.summary.replace(/^assistant: /, "");
  return s ? s[0].toUpperCase() + s.slice(1) : "Change";
}

export function HistoryPanel({
  rev,
  onClose,
  onEdit,
  onError,
}: {
  /** The project's current revision: the list reloads when it changes. */
  rev: number | null | undefined;
  onClose: () => void;
  onEdit: (p: EditResult) => void;
  onError: (e: AppError) => void;
}) {
  const [rows, setRows] = useState<HistoryRow[] | null>(null);
  const [picked, setPicked] = useState<number | null>(null);

  useEffect(() => {
    runCommand(commands.projectHistory(), setRows, onError);
  }, [rev, onError]);

  const chosen = rows?.find((r) => r.rev === picked) ?? null;

  return (
    <div
      className="modal-backdrop"
      onMouseDown={onClose}
      data-testid="history-panel"
    >
      <div
        className="modal history"
        role="dialog"
        aria-label="Project history"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header>
          <h3>History</h3>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <Icon name="close" size={16} />
          </button>
        </header>
        <p className="muted small">
          Every change to this project, newest first. Pick one to go back to
          how the project was before it; that is one more change you can undo.
        </p>
        {rows === null ? (
          <p className="muted">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="muted">No changes yet.</p>
        ) : (
          <ul className="history-list" data-testid="history-list">
            {rows.map((r) => (
              <li key={r.rev ?? 0}>
                <button
                  className={`history-row${r.rev === picked ? " on" : ""}`}
                  onClick={() => setPicked(r.rev === picked ? null : r.rev)}
                  data-testid="history-row"
                >
                  <span className={`who who-${r.source}`}>{r.who}</span>
                  <span className="history-summary">{shortSummary(r)}</span>
                  <span className="muted small">
                    {formatWhen(r.time ?? 0)}
                  </span>
                </button>
                {r.rev === picked && chosen && (
                  <div className="history-actions">
                    <button
                      className="btn"
                      disabled={!chosen.canRestore}
                      title={
                        chosen.canRestore
                          ? undefined
                          : "Older history is cleared after 30 days or 2,000 changes."
                      }
                      onClick={() => {
                        onEdit(commands.restoreBefore(chosen.rev));
                        setPicked(null);
                      }}
                      data-testid="history-restore"
                    >
                      Restore to before this
                    </button>
                    <span className="muted small">
                      {new Date(chosen.time ?? 0).toLocaleString()}
                    </span>
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
