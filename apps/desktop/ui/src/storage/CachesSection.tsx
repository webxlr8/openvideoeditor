// Settings → Storage, derived data (add-background-tasks-and-media-derivatives 6.3): one row per kind (thumbnails,
// skim sheets, proxies, transcripts, the audio peak cache) with its size in the open project and across the library,
// and Clear. Clearing cancels that kind's background tasks first, then removes only those cache files: never media
// and never the project. The open project's "Use proxies" setting sits with the proxies row.
import { useCallback, useEffect, useState } from "react";

import type { StorageView_Serialize } from "../bindings";
import { commands } from "../bridge";
import { type ErrorSink, runCommand } from "../notify";
import { sizeText } from "./ModelsSection";

export function CachesSection({ onError }: { onError: ErrorSink }) {
  const [view, setView] = useState<StorageView_Serialize | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [clearing, setClearing] = useState<string | null>(null);

  const refresh = useCallback(
    () => runCommand(commands.storageUsage(), setView, onError),
    [onError],
  );
  useEffect(() => {
    refresh();
  }, [refresh]);

  const clear = (kind: string, label: string) => {
    setClearing(kind);
    runCommand(
      commands.clearCache(kind),
      (r) => {
        setClearing(null);
        setView(r.view);
        const stopped =
          r.cancelledTasks > 0
            ? ` ${r.cancelledTasks} running task${r.cancelledTasks === 1 ? " was" : "s were"} cancelled first.`
            : "";
        const left =
          r.left > 0 ? ` ${r.left} file${r.left === 1 ? " is" : "s are"} in use and stay for now.` : "";
        setMessage(`${label} cleared: ${sizeText(r.freedBytes)} freed.${stopped}${left}`);
      },
      (e) => {
        setClearing(null);
        onError(e);
      },
    );
  };

  const setProxies = (on: boolean) => {
    runCommand(commands.setUseProxies(on), () => refresh(), onError);
  };

  const project = view?.project ?? null;
  return (
    <div data-testid="caches-section">
      <p className="muted small">
        Thumbnails, skim sheets, proxies, transcripts and waveforms are made
        from your media and kept in each project. Clearing one frees its space;
        it is made again when needed. Your media files and the project itself
        are never touched.
      </p>
      {view === null && <p className="muted small">Measuring…</p>}
      <ul className="gen-jobs" data-testid="caches-list">
        {(view?.rows ?? []).map((r) => (
          <li key={r.kind} className="gen-job" data-testid={`cache-${r.kind}`}>
            <div className="gen-job-text">
              <span className="gen-job-prompt">{r.label}</span>
              <span className="muted small">
                {project !== null && (
                  <>
                    This project:{" "}
                    <span data-testid={`cache-size-${r.kind}`}>
                      {sizeText(r.projectBytes)}
                    </span>{" "}
                    ·{" "}
                  </>
                )}
                All projects:{" "}
                <span data-testid={`cache-total-${r.kind}`}>
                  {sizeText(r.totalBytes)}
                </span>
              </span>
              {r.kind === "proxies" && project !== null && (
                <label className="gen-check small">
                  <input
                    type="checkbox"
                    checked={view?.useProxies ?? false}
                    onChange={(e) => setProxies(e.target.checked)}
                    data-testid="use-proxies"
                  />{" "}
                  Use proxies in “{project}”: 4K, 10-bit and slow-to-decode
                  clips play from 1080p copies. Export always uses the
                  originals.
                </label>
              )}
            </div>
            {project !== null && (
              <button
                className="btn"
                disabled={clearing !== null || r.projectBytes === 0}
                onClick={() => clear(r.kind, r.label)}
                data-testid={`cache-clear-${r.kind}`}
              >
                {clearing === r.kind ? "Clearing…" : "Clear"}
              </button>
            )}
          </li>
        ))}
      </ul>
      {view !== null && project === null && (
        <p className="muted small">Open a project to clear its stored data.</p>
      )}
      {message && (
        <p className="small" data-testid="caches-message">
          {message}
        </p>
      )}
    </div>
  );
}
