// The History panel's named versions: save the current timeline under a name, compare a version with the timeline
// now, and restore it as one new revision (Undo brings back what was there). Versions survive history pruning.

import { useEffect, useState } from "react";
import "./versions.css";
import type { AppError, VersionChange, VersionView_Serialize as VersionView } from "./bindings";
import { commands } from "./bridge";
import type { EditResult } from "./edit";
import { runCommand } from "./notify";
import { formatWhen } from "./timecode";

export function VersionsSection({
  rev,
  onEdit,
  onError,
}: {
  /** The project's current revision: the list reloads when it changes. */
  rev: number | null | undefined;
  onEdit: (p: EditResult) => void;
  onError: (e: AppError) => void;
}) {
  const [versions, setVersions] = useState<VersionView[] | null>(null);
  const [name, setName] = useState("");
  const [compared, setCompared] = useState<{ name: string; changes: VersionChange[] } | null>(null);

  useEffect(() => {
    runCommand(commands.versionList(), setVersions, onError);
  }, [rev, onError]);

  const save = () => {
    const n = name.trim();
    if (!n) return;
    runCommand(
      commands.versionSave(n, null),
      (list) => {
        setVersions(list);
        setName("");
      },
      onError,
    );
  };

  return (
    <section className="versions" data-testid="versions">
      <h4>Versions</h4>
      <form
        className="version-save"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Name this version, e.g. client v2"
          aria-label="Version name"
          data-testid="version-name"
        />
        <button className="btn" type="submit" disabled={!name.trim()} data-testid="version-save">
          Save version
        </button>
      </form>
      {versions && versions.length > 0 && (
        <ul className="version-list" data-testid="version-list">
          {versions.map((v) => (
            <li key={v.name} data-testid="version-row">
              <span className="version-name">{v.name}</span>
              <span className="muted small">
                {v.who} · {formatWhen(v.createdAt ?? 0)}
                {v.note ? ` · ${v.note}` : ""}
              </span>
              <span className="version-actions">
                <button
                  className="btn"
                  data-testid="version-compare"
                  onClick={() =>
                    compared?.name === v.name
                      ? setCompared(null)
                      : runCommand(commands.versionDiff(v.name, null), (changes) => setCompared({ name: v.name, changes }), onError)
                  }
                >
                  {compared?.name === v.name ? "Hide changes" : "Compare with now"}
                </button>
                <button className="btn" data-testid="version-restore" onClick={() => onEdit(commands.versionRestore(v.name))}>
                  Restore
                </button>
              </span>
              {compared?.name === v.name && (
                <ul className="version-changes" data-testid="version-changes">
                  {compared.changes.length === 0 ? (
                    <li className="muted small">The timeline is the same as this version.</li>
                  ) : (
                    compared.changes.map((c) => (
                      <li key={`${c.entity}-${c.id}`} className={`change-${c.class}`} data-testid="version-change">
                        {c.line}
                      </li>
                    ))
                  )}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
