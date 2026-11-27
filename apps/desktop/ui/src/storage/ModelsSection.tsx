// Settings → Storage, Models: the local machine-learning models (speech recognition today), each with its purpose,
// size and folder, and Remove. Removing a model that a running job uses waits for the job, and says so.
import { useCallback, useEffect, useState } from "react";

import type {
  LocalModelsChanged_Serialize,
  LocalModels_Serialize,
} from "../bindings";
import { commands, events } from "../bridge";
import { type ErrorSink, runCommand, unlisten } from "../notify";

/** Bytes as MB below 1 GB, else GB, with one decimal. */
export function sizeText(bytes: number | null | undefined): string {
  if (bytes == null) return "…";
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${(bytes / 1e6).toFixed(1)} MB`;
}

function outcome(r: LocalModelsChanged_Serialize): string {
  if (r.status === "waiting")
    return `${r.id} is in use. It will be removed when the running job ends or is cancelled.`;
  if (r.status === "notInstalled") return `${r.id} is not installed.`;
  return `Removed ${r.id}: ${sizeText(r.freedBytes)} freed${r.freeBytes != null ? `, ${sizeText(r.freeBytes)} free` : ""}.`;
}

export function ModelsSection({ onError }: { onError: ErrorSink }) {
  const [list, setList] = useState<LocalModels_Serialize | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [waiting, setWaiting] = useState<string[]>([]);

  const refresh = useCallback(
    () => runCommand(commands.localModels(), setList, onError),
    [onError],
  );
  useEffect(() => {
    refresh();
    const off = events.localModelsChanged.listen((e) => {
      setWaiting((w) => w.filter((id) => id !== e.payload.id));
      setMessage(outcome(e.payload));
      refresh();
    });
    return () => unlisten(off);
  }, [refresh]);

  const remove = (id: string) => {
    runCommand(
      commands.removeLocalModel(id),
      (r) => {
        if (r.status === "waiting") setWaiting((w) => [...w, id]);
        setMessage(outcome(r));
        refresh();
      },
      onError,
    );
  };

  const installed = (list?.models ?? []).filter((m) => m.installed);
  const used = installed.reduce((sum, m) => sum + (m.bytes ?? 0), 0);
  return (
    <div data-testid="models-section">
      <p className="muted small">
        Models download when a feature first needs one; none ship with the app.
        Models larger than 50 MB download only after you agree.
        {list && (
          <>
            {" "}
            Installed: {sizeText(used)}
            {list.freeBytes != null && (
              <> · {sizeText(list.freeBytes)} free on this disk</>
            )}
            .
          </>
        )}
      </p>
      {list === null && <p className="muted small">Reading the model store…</p>}
      <ul className="gen-jobs" data-testid="models-list">
        {(list?.models ?? []).map((m) => (
          <li key={m.id} className="gen-job" data-testid={`model-${m.id}`}>
            <div className="gen-job-text">
              <span className="gen-job-prompt">{m.purpose}</span>
              <span className="muted small">
                <span className="mono">{m.id}</span> · {sizeText(m.bytes)} ·
                licence {m.licence} ·{" "}
                <span data-testid={`model-state-${m.id}`}>
                  {m.installed
                    ? waiting.includes(m.id)
                      ? "removing after the job"
                      : m.inUse
                        ? "in use"
                        : "installed"
                    : "not installed"}
                </span>
              </span>
              {m.location && (
                <span className="muted small mono">{m.location}</span>
              )}
            </div>
            {m.installed && (
              <button
                className="btn"
                disabled={waiting.includes(m.id)}
                onClick={() => remove(m.id)}
                data-testid={`model-remove-${m.id}`}
              >
                {waiting.includes(m.id) ? "Waiting…" : "Remove"}
              </button>
            )}
          </li>
        ))}
      </ul>
      {message && (
        <p className="small" data-testid="models-message">
          {message}
        </p>
      )}
      {list && <p className="muted small mono">{list.root}</p>}
    </div>
  );
}
