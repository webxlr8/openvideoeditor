// Settings → Storage, Models: the local machine-learning models (speech recognition, the inference runtime), each
// with its purpose, size and folder, Install and Remove. Install names the host before anything downloads and shows
// progress with Cancel. Removing a model that a running job uses waits for the job, and says so.
import { useCallback, useEffect, useState } from "react";

import type {
  LocalModelInstallProgress_Serialize,
  LocalModelsChanged_Serialize,
  LocalModels_Serialize,
} from "../bindings";
import { commands, events } from "../bridge";
import { type ErrorSink, runCommand, unlisten } from "../notify";

/** Bytes as MB below 1 GB, else GB, with one decimal. */
export function sizeText(bytes: number | null | undefined): string {
  if (bytes == null) return "…";
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  // From 500 MB up, gigabytes with two decimals (the visual search model reads "0.63 GB", as `ove models` shows it).
  if (bytes >= 5e8) return `${(bytes / 1e9).toFixed(2)} GB`;
  return `${(bytes / 1e6).toFixed(1)} MB`;
}

function outcome(r: LocalModelsChanged_Serialize): string {
  if (r.status === "waiting")
    return `${r.id} is in use. It will be removed when the running job ends or is cancelled.`;
  if (r.status === "notInstalled") return `${r.id} is not installed.`;
  return `Removed ${r.id}: ${sizeText(r.freedBytes)} freed${r.freeBytes != null ? `, ${sizeText(r.freeBytes)} free` : ""}.`;
}

function percent(p: LocalModelInstallProgress_Serialize): number {
  if (!p.totalBytes) return 0;
  return Math.floor(((p.doneBytes ?? 0) * 100) / p.totalBytes);
}

export function ModelsSection({ onError }: { onError: ErrorSink }) {
  const [list, setList] = useState<LocalModels_Serialize | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [waiting, setWaiting] = useState<string[]>([]);
  const [downloads, setDownloads] = useState<
    Record<string, LocalModelInstallProgress_Serialize>
  >({});

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
    const offInstall = events.localModelInstallProgress.listen((e) => {
      const p = e.payload;
      setDownloads((d) => {
        const next = { ...d };
        if (p.finished) delete next[p.id];
        else next[p.id] = p;
        return next;
      });
      if (p.finished) {
        setMessage(
          p.error
            ? p.error === "cancelled"
              ? `The download of ${p.id} was cancelled.`
              : `Could not install ${p.id}: ${p.error}`
            : `Installed ${p.id}.`,
        );
        refresh();
      }
    });
    return () => {
      unlisten(off);
      unlisten(offInstall);
    };
  }, [refresh]);

  const install = (id: string) => {
    runCommand(commands.installLocalModel(id), () => {}, onError);
  };
  const cancel = (id: string) => {
    runCommand(commands.cancelLocalModelInstall(id), () => {}, onError);
  };

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
        Models larger than 50 MB, and the inference runtime, download only after
        you agree.
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
              {!m.installed && !downloads[m.id] && m.host && (
                <span
                  className="muted small"
                  data-testid={`model-source-${m.id}`}
                >
                  Downloads {sizeText(m.downloadBytes)} from{" "}
                  <span className="mono">{m.host}</span>
                </span>
              )}
              {downloads[m.id] && (
                <span className="small" data-testid={`model-progress-${m.id}`}>
                  Downloading from{" "}
                  <span className="mono">{downloads[m.id].host}</span>:{" "}
                  {percent(downloads[m.id])}% (
                  {sizeText(downloads[m.id].doneBytes)} of{" "}
                  {sizeText(downloads[m.id].totalBytes)})
                </span>
              )}
            </div>
            {!m.installed &&
              (downloads[m.id] ? (
                <button
                  className="btn"
                  onClick={() => cancel(m.id)}
                  data-testid={`model-cancel-${m.id}`}
                >
                  Cancel
                </button>
              ) : (
                <button
                  className="btn"
                  onClick={() => install(m.id)}
                  data-testid={`model-install-${m.id}`}
                >
                  Install
                </button>
              ))}
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
