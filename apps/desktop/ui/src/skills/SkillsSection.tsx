// Settings → Skills: the playbooks agents read. Installed (builtin and catalog skills), Community (the catalog the
// person set, with install and update, each shown before it happens), and Local (the person's own skills, the ones
// the assistant saved, and drafts). Every skill can be opened in the editor and exported to other agents.
import { useCallback, useEffect, useState } from "react";

import type {
  CatalogView,
  SkillExportOutcome_Serialize as SkillExportOutcome,
  SkillInstallPreview,
  SkillRow_Serialize as SkillRow,
  SkillUpdateOffer,
  SkillsView_Serialize as SkillsView,
} from "../bindings";
import { commands } from "../bridge";
import { type ErrorSink, runCommand } from "../notify";
import "./skills.css";

type Tab = "installed" | "community" | "local";

const NEW_SKILL = `---
name: My skill
description: What this playbook does, and when an agent should use it.
---

1. First step.
2. Second step.
`;

/** What the editor shows: an existing skill (read-only when builtin), or a new one. */
type Editing = { id: string | null; text: string; readOnly: boolean; drafted: boolean };

function Badges({ s, update }: { s: SkillRow; update?: string }) {
  return (
    <span className="skill-badges">
      <span className="skill-badge" data-testid={`skill-source-${s.id}`}>
        {s.source}
      </span>
      {s.invalid && (
        <span className="skill-badge warn" data-testid={`skill-invalid-${s.id}`}>
          invalid: {s.invalid}
        </span>
      )}
      {s.modified && (
        <span className="skill-badge" data-testid={`skill-modified-${s.id}`}>
          modified
        </span>
      )}
      {update === "available" && <span className="skill-badge accent">update available</span>}
      {update === "modified" && <span className="skill-badge accent">update available as a copy</span>}
    </span>
  );
}

function ExportMenu({
  skill,
  view,
  onError,
}: {
  skill: SkillRow;
  view: SkillsView;
  onError: ErrorSink;
}) {
  const [open, setOpen] = useState(false);
  const [results, setResults] = useState<SkillExportOutcome[]>([]);
  const run = (targets: string[], overwrite: boolean) =>
    runCommand(commands.skillExport(skill.id, targets, overwrite), setResults, onError);
  return (
    <span className="skill-export">
      <button className="btn" data-testid={`skill-export-${skill.id}`} onClick={() => setOpen((o) => !o)}>
        Export…
      </button>
      {open && (
        <div className="skill-export-menu" data-testid={`skill-export-menu-${skill.id}`}>
          <p className="muted small">
            Copies the skill as <span className="mono">ove-{skill.id}</span> into the agent's skills folder.
          </p>
          {view.targets.map((t) => (
            <div key={t.id} className="skill-export-row">
              <button
                className="btn"
                disabled={!t.available}
                data-testid={`skill-export-to-${t.id}`}
                onClick={() => run([t.id], false)}
              >
                {t.name}
              </button>
              {!t.available && (
                <span className="muted small" data-testid={`skill-export-reason-${t.id}`}>
                  unavailable: {t.reason}
                </span>
              )}
            </div>
          ))}
          {results.map((r) => (
            <div key={r.target} className="small" data-testid={`skill-export-result-${r.target}`}>
              {r.ok ? (
                <>
                  {r.name}: exported to <span className="mono">{r.file}</span>
                </>
              ) : r.code === "export.modified" ? (
                <>
                  You changed the copy in {r.name}.{" "}
                  <button
                    className="btn"
                    data-testid={`skill-export-replace-${r.target}`}
                    onClick={() => run([r.target], true)}
                  >
                    Replace it
                  </button>
                </>
              ) : (
                <>
                  {r.name}: {r.message}
                </>
              )}
            </div>
          ))}
        </div>
      )}
    </span>
  );
}

function Editor({
  editing,
  onSave,
  onCancel,
}: {
  editing: Editing;
  onSave: (text: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(editing.text);
  return (
    <div className="skill-editor" data-testid="skill-editor">
      <h4>{editing.id ? (editing.readOnly ? `${editing.id} (builtin, read-only)` : editing.id) : "New skill"}</h4>
      <textarea
        className="mono"
        rows={18}
        value={text}
        readOnly={editing.readOnly}
        data-testid="skill-editor-text"
        onChange={(e) => setText(e.target.value)}
      />
      <div className="skill-actions">
        {!editing.readOnly && (
          <button className="btn primary" data-testid="skill-editor-save" onClick={() => onSave(text)}>
            Save
          </button>
        )}
        <button className="btn" data-testid="skill-editor-close" onClick={onCancel}>
          {editing.readOnly ? "Close" : "Cancel"}
        </button>
      </div>
    </div>
  );
}

function Community({
  view,
  onError,
  onChanged,
}: {
  view: SkillsView;
  onError: ErrorSink;
  onChanged: () => void;
}) {
  const [url, setUrl] = useState(view.catalogUrl);
  const [catalog, setCatalog] = useState<CatalogView | null>(null);
  const [preview, setPreview] = useState<SkillInstallPreview | null>(null);
  const [offer, setOffer] = useState<SkillUpdateOffer | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(() => {
    if (!view.catalogUrl) return;
    runCommand(commands.skillsCatalog(), setCatalog, (e) => {
      setCatalog(null);
      setMessage(e.message);
    });
  }, [view.catalogUrl]);
  useEffect(load, [load]);

  const saveUrl = () =>
    runCommand(
      commands.skillsSetCatalog(url.trim()),
      () => {
        setMessage(null);
        onChanged();
      },
      onError,
    );
  const fail = (e: { message: string }) => setMessage(e.message);

  if (!view.catalogUrl) {
    return (
      <div data-testid="skills-community-unset">
        <p className="muted small">
          No community catalog is set. A catalog is a list of skills that someone publishes at an HTTPS address, each
          pinned by its SHA-256. Paste its address here to browse it. Nothing is downloaded until you do, and every
          install is shown to you first.
        </p>
        <div className="skill-actions">
          <input
            className="input"
            placeholder="https://…/catalog.json"
            value={url}
            data-testid="skills-catalog-url"
            onChange={(e) => setUrl(e.target.value)}
          />
          <button className="btn" data-testid="skills-catalog-save" onClick={saveUrl} disabled={!url.trim()}>
            Use this catalog
          </button>
        </div>
      </div>
    );
  }

  return (
    <div data-testid="skills-community">
      <p className="muted small">
        Catalog: <span className="mono">{view.catalogUrl}</span>{" "}
        <button className="btn" data-testid="skills-catalog-refresh" onClick={load}>
          Refresh
        </button>{" "}
        <button
          className="btn"
          data-testid="skills-catalog-clear"
          onClick={() => runCommand(commands.skillsSetCatalog(""), onChanged, onError)}
        >
          Remove address
        </button>
      </p>
      <p className="muted small">
        Catalog skills are text from someone else. They can't run anything or give an agent more tools, but read one
        before you install it.
      </p>
      {message && (
        <p className="small" data-testid="skills-community-message">
          {message}
        </p>
      )}
      {catalog === null && !message && <p className="muted small">Loading the catalog…</p>}
      <ul className="gen-jobs">
        {(catalog?.entries ?? []).map((e) => {
          const installed = view.skills.find((s) => s.kind === "catalog" && s.id === e.id);
          return (
            <li key={e.id} className="gen-job" data-testid={`catalog-${e.id}`}>
              <div className="gen-job-text">
                <span className="gen-job-prompt">{e.name}</span>
                <span className="muted small">
                  <span className="mono">{e.id}</span> · {e.description}
                </span>
              </div>
              {installed ? (
                <span className="muted small">installed</span>
              ) : (
                <button
                  className="btn"
                  data-testid={`catalog-install-${e.id}`}
                  onClick={() => runCommand(commands.skillInstallPreview(e.id), setPreview, fail)}
                >
                  Install…
                </button>
              )}
            </li>
          );
        })}
      </ul>
      {(catalog?.installed ?? [])
        .filter((r) => r.state === "available" || r.state === "modified")
        .map((r) => (
          <div key={r.id} className="skill-update" data-testid={`skill-update-${r.id}`}>
            {r.state === "available"
              ? `An update to ${r.id} is available.`
              : `An update to ${r.id} is available. You edited it, so your version stays and the update comes as a copy.`}{" "}
            <button
              className="btn"
              data-testid={`skill-update-review-${r.id}`}
              onClick={() => runCommand(commands.skillUpdateOffer(r.id), setOffer, fail)}
            >
              Review the changes
            </button>
          </div>
        ))}
      {preview && (
        <div className="skill-review" data-testid="skill-install-preview">
          <h4>Install {preview.name}?</h4>
          <p className="muted small">
            From <span className="mono">{preview.catalogUrl}</span>. It matches its pinned SHA-256{" "}
            <span className="mono">{preview.sha256.slice(0, 16)}…</span> and will be installed as{" "}
            <span className="mono">{preview.installAs}</span>.
          </p>
          <pre className="skill-text" data-testid="skill-install-text">
            {preview.text}
          </pre>
          <div className="skill-actions">
            <button
              className="btn primary"
              data-testid="skill-install-confirm"
              onClick={() =>
                runCommand(
                  commands.skillInstall(preview.id, preview.installAs, preview.sha256),
                  (s) => {
                    setPreview(null);
                    setMessage(`Installed ${s.id}.`);
                    onChanged();
                    load();
                  },
                  fail,
                )
              }
            >
              Install
            </button>
            <button className="btn" data-testid="skill-install-cancel" onClick={() => setPreview(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {offer && (
        <div className="skill-review" data-testid="skill-update-offer">
          <h4>Update {offer.id}</h4>
          <p className="muted small">
            {offer.asCopy
              ? `Your edited ${offer.id} stays as it is. The update will be installed as ${offer.installAs}.`
              : `This replaces ${offer.id} with the catalog's new version.`}
          </p>
          <pre className="skill-text skill-diff" data-testid="skill-update-diff">
            {offer.diff
              .filter((l) => l.op !== " ")
              .map((l, i) => (
                <div key={i} className={l.op === "+" ? "add" : "del"}>
                  {l.op}
                  {l.text}
                </div>
              ))}
          </pre>
          <div className="skill-actions">
            <button
              className="btn primary"
              data-testid="skill-update-accept"
              onClick={() =>
                runCommand(
                  commands.skillUpdateAccept(offer.id, offer.sha256),
                  (s) => {
                    setOffer(null);
                    setMessage(offer.asCopy ? `Installed the update as ${s.id}.` : `Updated ${s.id}.`);
                    onChanged();
                    load();
                  },
                  fail,
                )
              }
            >
              Accept the update
            </button>
            <button className="btn" data-testid="skill-update-keep" onClick={() => setOffer(null)}>
              Keep the current version
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export function SkillsSection({ onError }: { onError: ErrorSink }) {
  const [view, setView] = useState<SkillsView | null>(null);
  const [tab, setTab] = useState<Tab>("installed");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [drafting, setDrafting] = useState(false);
  const [withChat, setWithChat] = useState(false);

  const refresh = useCallback(() => runCommand(commands.skillsView(), setView, onError), [onError]);
  useEffect(refresh, [refresh]);

  const draft = () => {
    setDrafting(true);
    setMessage(null);
    runCommand(
      commands.skillDraft(withChat).finally(() => setDrafting(false)),
      (text) => {
        setEditing({ id: null, text, readOnly: false, drafted: true });
        setMessage("This is a draft. Read it and change what you need; nothing is saved until you click Save.");
      },
      (e) => setMessage(e.message),
    );
  };

  const open = (s: SkillRow) =>
    runCommand(
      commands.skillText(s.id),
      (text) => setEditing({ id: s.id, text, readOnly: s.builtin, drafted: false }),
      onError,
    );
  const save = (text: string) => {
    if (!editing) return;
    runCommand(
      commands.skillSave(editing.id, text, editing.drafted),
      (s) => {
        setEditing(null);
        setMessage(`Saved ${s.id}.`);
        refresh();
      },
      onError,
    );
  };
  const remove = (s: SkillRow) =>
    runCommand(
      commands.skillRemove(s.id),
      () => {
        setMessage(`Removed ${s.id}.`);
        refresh();
      },
      onError,
    );

  if (!view) return <p className="muted small">Reading the skills…</p>;
  const installed = view.skills.filter((s) => s.kind === "builtin" || s.kind === "catalog");
  const local = view.skills.filter((s) => s.kind !== "builtin" && s.kind !== "catalog");
  const rows = (list: SkillRow[]) => (
    <ul className="gen-jobs" data-testid={`skills-list-${tab}`}>
      {list.map((s) => (
        <li key={s.id} className="gen-job" data-testid={`skill-${s.id}`}>
          <div className="gen-job-text">
            <span className="gen-job-prompt">{s.name}</span>
            <span className="muted small">
              <span className="mono">{s.id}</span> · {s.description}
            </span>
            <Badges s={s} />
          </div>
          <div className="skill-actions">
            <button className="btn" data-testid={`skill-open-${s.id}`} onClick={() => open(s)}>
              {s.builtin ? "View" : "Edit"}
            </button>
            {!s.invalid && <ExportMenu skill={s} view={view} onError={onError} />}
            {!s.builtin && (
              <button className="btn" data-testid={`skill-remove-${s.id}`} onClick={() => remove(s)}>
                Remove
              </button>
            )}
          </div>
        </li>
      ))}
    </ul>
  );

  return (
    <div data-testid="skills-section">
      <p className="muted small">
        Skills are playbooks agents read before an edit: the assistant here, and agents connected through{" "}
        <span className="mono">ove mcp</span>. A skill is text only. It can't run anything, and it can't give an agent
        more tools.
      </p>
      {view.validCount > view.indexCap && (
        <p className="small warn" data-testid="skills-cap-warning">
          You have {view.validCount} skills. The assistant's list shows the first {view.indexCap}; it can still find
          the others with list_skills.
        </p>
      )}
      <div className="skill-tabs" role="tablist">
        {(
          [
            ["installed", "Installed"],
            ["community", "Community"],
            ["local", "Local"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={`btn${tab === id ? " active" : ""}`}
            data-testid={`skills-tab-${id}`}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      {message && (
        <p className="small" data-testid="skills-message">
          {message}
        </p>
      )}
      {editing && (
        <Editor key={editing.id ?? "new"} editing={editing} onSave={save} onCancel={() => setEditing(null)} />
      )}
      {tab === "installed" && rows(installed)}
      {tab === "community" && <Community view={view} onError={onError} onChanged={refresh} />}
      {tab === "local" && (
        <>
          <button
            className="btn"
            data-testid="skill-new"
            onClick={() => setEditing({ id: null, text: NEW_SKILL, readOnly: false, drafted: false })}
          >
            New skill
          </button>{" "}
          <button className="btn" data-testid="skill-draft" onClick={draft} disabled={drafting}>
            {drafting ? "Drafting…" : "Draft from the open project"}
          </button>{" "}
          <label className="small">
            <input
              type="checkbox"
              checked={withChat}
              data-testid="skill-draft-chat"
              onChange={(e) => setWithChat(e.target.checked)}
            />{" "}
            include the chat
          </label>
          <p className="muted small">
            Drafting sends the open project's timeline, its edit history and the presets it uses (and the chat, if you
            include it) to the assistant, which writes a playbook to rebuild the edit and reuse its style. You review the
            draft; nothing is saved until you click Save.
          </p>
          {local.length === 0 && (
            <p className="muted small">
              No skills of your own yet. Write one, or ask the assistant to save a way of working as a skill.
            </p>
          )}
          {rows(local)}
        </>
      )}
      <p className="muted small">
        Folder: <span className="mono">{view.root}</span>
      </p>
    </div>
  );
}
