// The status bar's Background Tasks button and popover (add-background-tasks-and-media-derivatives 6.1): running,
// waiting and paused tasks with progress, the reason work is paused with "Run anyway", Cancel per task, and pause or
// resume all. The button is not shown while no background work exists. Updates arrive as `tasksChanged` events (at
// most four a second); the popover never polls.
import { useEffect, useState } from "react";

import type { TaskView_Serialize, TasksView_Serialize } from "../bindings";
import { commands, events } from "../bridge";
import { type ErrorSink, runCommand, unlisten } from "../notify";
import "./tasks.css";

/** What a task's state line says. */
export function stateText(t: TaskView_Serialize, labels: Record<string, string>): string {
  if (t.state === "running") {
    const pct = t.progress != null ? ` ${Math.round(t.progress * 100)}%` : "";
    return t.runningAlone ? `Running alone${pct} (needs a lot of memory)` : `Running${pct}`;
  }
  if (t.state === "paused") {
    const why = t.pauseReason ? (labels[t.pauseReason] ?? t.pauseReason) : null;
    return why ? `Paused: ${why}` : "Paused";
  }
  return "Waiting";
}

/** An entry for the export queue (its own list lives elsewhere): how many jobs wait or run, and how to open it. */
export interface ExportsEntry {
  count: number;
  onOpen: () => void;
}

export function TasksButton({
  onError,
  exports,
}: {
  onError: ErrorSink;
  /** Shows an "Exports" row that opens the export queue; the button also shows while it has jobs. */
  exports?: ExportsEntry;
}) {
  const [view, setView] = useState<TasksView_Serialize | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    // An update that arrives before the first list answers is newer than it.
    let updated = false;
    const off = events.tasksChanged.listen((e) => {
      updated = true;
      setView(e.payload);
    });
    runCommand(
      commands.listTasks(),
      (v) => {
        if (!updated) setView(v);
      },
      onError,
    );
    return () => unlisten(off);
  }, [onError]);

  const tasks = view?.tasks ?? [];
  const userPaused = (view?.paused ?? []).some((p) => p.reason === "user");
  const exportJobs = exports?.count ?? 0;
  if (tasks.length === 0 && !userPaused && exportJobs === 0) return null;

  const labels = Object.fromEntries((view?.paused ?? []).map((p) => [p.reason, p.label]));
  const running = tasks.filter((t) => t.state === "running").length;
  const reasons = (view?.paused ?? []).filter((p) => p.reason !== "user");
  const cancel = (id: number) => runCommand(commands.cancelTask(id), () => {}, onError);
  const runAnyway = (reason: string, on: boolean) =>
    runCommand(commands.runAnyway(reason, on), () => {}, onError);
  const pauseAll = (paused: boolean) =>
    runCommand(commands.pauseTasks(paused), () => {}, onError);

  return (
    <div className="bt-wrap">
      <button
        className={`btn bt-button${running > 0 ? " busy" : ""}`}
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        title="Background tasks"
        data-testid="tasks-button"
      >
        <span className="bt-dot" aria-hidden />
        {tasks.length} task{tasks.length === 1 ? "" : "s"}
        {reasons.length > 0 || userPaused ? " · paused" : ""}
      </button>
      {open && (
        <div className="bt-popover" role="dialog" aria-label="Background tasks" data-testid="tasks-popover">
          <div className="bt-head">
            <strong>Background tasks</strong>
            <button
              className="btn"
              onClick={() => pauseAll(!userPaused)}
              data-testid="tasks-pause-all"
            >
              {userPaused ? "Resume all" : "Pause all"}
            </button>
          </div>
          {reasons.map((p) => (
            <div key={p.reason} className="bt-reason" data-testid={`tasks-reason-${p.reason}`}>
              <span>Paused: {p.label}</span>
              <button
                className="btn"
                onClick={() => runAnyway(p.reason, true)}
                data-testid={`tasks-run-anyway-${p.reason}`}
              >
                Run anyway
              </button>
            </div>
          ))}
          {(view?.runAnyway ?? []).map((r) => (
            <div key={r} className="bt-reason muted small" data-testid={`tasks-overridden-${r}`}>
              <span>Running anyway ({r})</span>
              <button className="btn" onClick={() => runAnyway(r, false)}>
                Pause again
              </button>
            </div>
          ))}
          {exports && (
            <div className="bt-reason" data-testid="tasks-exports">
              <span>
                Exports{exportJobs > 0 ? ` (${exportJobs})` : ""}
              </span>
              <button
                className="btn"
                onClick={() => {
                  setOpen(false);
                  exports.onOpen();
                }}
                data-testid="tasks-open-exports"
              >
                Open
              </button>
            </div>
          )}
          {tasks.length === 0 && <p className="muted small">Nothing is waiting.</p>}
          <ul className="bt-list">
            {tasks.map((t) => (
              <li key={t.id} className={`bt-task ${t.state}`} data-testid={`task-${t.id}`}>
                <div className="bt-text">
                  <span className="bt-label">{t.label}</span>
                  <span className="muted small" data-testid={`task-state-${t.id}`}>
                    {stateText(t, labels)}
                  </span>
                  {t.state === "running" && t.progress != null && (
                    <span className="bt-bar">
                      <span style={{ width: `${Math.round(t.progress * 100)}%` }} />
                    </span>
                  )}
                </div>
                {t.priority !== "export" && (
                  <button
                    className="btn"
                    onClick={() => cancel(t.id)}
                    aria-label={`Cancel ${t.label}`}
                    data-testid={`task-cancel-${t.id}`}
                  >
                    Cancel
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
