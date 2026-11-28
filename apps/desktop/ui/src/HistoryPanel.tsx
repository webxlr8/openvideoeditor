// The project's history: every revision, newest first, with who made it (you, the assistant, the command line, an
// MCP agent, a template or a generation) and when. "Restore to before this" puts the project back as it was before
// a change, as one new revision, so it can itself be undone. Consecutive changes by the same agent are grouped, and
// a group offers "Undo this agent's changes" (change add-agent-chat-context-and-activity, design D6).

import { Fragment, useEffect, useState } from "react";
import type { AppError, HistoryRow, UndoConflictRow } from "./bindings";
import { commands } from "./bridge";
import type { EditResult } from "./edit";
import { Icon } from "./icons";
import { runCommand } from "./notify";
import { formatWhen } from "./timecode";
import "./history.css";

/** Kinds of actor whose changes "Undo this agent's changes" can take back. */
const AGENTS = new Set(["mcp", "cli", "assistant"]);

/** Consecutive rows by one actor (newest first, like the rows). */
export interface HistoryGroup {
  key: string;
  name: string;
  rows: HistoryRow[];
  /** The agent's kind and session, when its changes can be undone as a whole. */
  agent: { kind: string; session: string; turn: string | null } | null;
}

/** Groups consecutive rows by who made them (an agent session, or the source when no actor was recorded). */
export function groupHistory(rows: HistoryRow[]): HistoryGroup[] {
  const groups: HistoryGroup[] = [];
  for (const r of rows) {
    const key = r.actorKind && r.session ? `${r.actorKind}:${r.session}` : `who:${r.who}`;
    const last = groups[groups.length - 1];
    if (last && last.key === key) {
      last.rows.push(r);
      continue;
    }
    const agent = r.actorKind && r.session && AGENTS.has(r.actorKind) ? { kind: r.actorKind, session: r.session, turn: r.turn ?? null } : null;
    groups.push({ key, name: r.actorName ?? r.who, rows: [r], agent });
  }
  // An assistant group made in one turn undoes that turn; otherwise the whole session.
  for (const g of groups)
    if (g.agent) g.agent.turn = g.agent.kind === "assistant" && g.rows.every((r) => r.turn && r.turn === g.rows[0].turn) ? (g.rows[0].turn ?? null) : null;
  return groups;
}

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
  // The group whose undo was refused, and why.
  const [refused, setRefused] = useState<{ key: string; conflicts: UndoConflictRow[] } | null>(null);

  const undoAgent = (g: HistoryGroup, at: string) => {
    if (!g.agent) return;
    const { kind, session, turn } = g.agent;
    setRefused(null);
    runCommand(
      commands.undoAgentChanges(kind, session, turn),
      (r) => {
        if (r.view) onEdit(Promise.resolve({ status: "ok", data: r.view }));
        else setRefused({ key: at, conflicts: r.conflicts });
      },
      onError,
    );
  };

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
            {groupHistory(rows).map((g, gi) => (
              <Fragment key={`${g.key}-${gi}`}>
                <li className="history-group" data-testid="history-group" data-actor={g.key}>
                  <span className="history-group-name">{g.name}</span>
                  <span className="muted small">
                    {g.rows.length} change{g.rows.length === 1 ? "" : "s"}
                  </span>
                  {g.agent && (
                    <button
                      className="btn small"
                      title="Revert only this agent's changes, as one change you can undo. Your edits and other agents' stay."
                      onClick={() => undoAgent(g, `${g.key}-${gi}`)}
                      data-testid="history-undo-agent"
                    >
                      Undo this agent's changes
                    </button>
                  )}
                </li>
                {refused?.key === `${g.key}-${gi}` && (
                  <li className="history-conflicts" role="alert" data-testid="history-conflicts">
                    Not undone: later edits changed the same {refused.conflicts.length === 1 ? "field" : "fields"}.
                    Nothing was changed.
                    <ul>
                      {refused.conflicts.map((c, i) => (
                        <li key={i}>
                          {c.item} · {c.field} — changed later by “{c.summary}”
                        </li>
                      ))}
                    </ul>
                  </li>
                )}
                {g.rows.map((r) => (
              <li key={r.rev ?? 0}>
                <button
                  className={`history-row${r.rev === picked ? " on" : ""}`}
                  onClick={() => setPicked(r.rev === picked ? null : r.rev)}
                  data-testid="history-row"
                >
                  <span className={`who who-${r.source}`}>{r.who}</span>
                  {r.client ? (
                    <span className="muted small" data-testid="history-client">
                      {r.client}
                    </span>
                  ) : null}
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
              </Fragment>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
