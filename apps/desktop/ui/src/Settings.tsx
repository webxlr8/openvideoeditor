// Settings (change 13): library, generation provider keys, assistant key, agents and the command line, shortcuts,
// privacy. Keys go to the system credential store through the core; nothing here is stored in plain files.

import { useEffect, useState } from "react";
import type {
  AgentStatus_Serialize,
  AppError,
  AppInfo_Serialize,
  CrashReport,
  UpdateInfo_Serialize as UpdateInfo,
} from "./bindings";
import { commands } from "./bridge";
import { type Catalog, KeyEntry, loadCatalog } from "./Generate";
import { Icon } from "./icons";
import {
  REBINDABLE,
  RESERVED,
  bindingOf,
  keyLabel,
  resetBindings,
  setBinding,
} from "./keys";

const SECTIONS = [
  { id: "library", label: "Library" },
  { id: "generation", label: "Generation" },
  { id: "assistant", label: "Assistant" },
  { id: "agents", label: "Agents & command line" },
  { id: "shortcuts", label: "Shortcuts" },
  { id: "privacy", label: "Privacy" },
  { id: "about", label: "About & updates" },
] as const;
type Section = (typeof SECTIONS)[number]["id"];

function Copyable({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <div className="set-code">
      <code>{text}</code>
      <button
        className="btn"
        onClick={() => {
          void navigator.clipboard?.writeText(text).then(() => {
            setDone(true);
            setTimeout(() => setDone(false), 1500);
          });
        }}
      >
        {done ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function ShortcutsSection() {
  const [, force] = useState(0);
  const [capturing, setCapturing] = useState<string | null>(null);
  useEffect(() => {
    if (!capturing) return;
    const on = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return setCapturing(null);
      if (
        e.metaKey ||
        e.ctrlKey ||
        e.altKey ||
        RESERVED.has(e.key) ||
        e.key.length !== 1
      )
        return;
      setBinding(capturing, e.key);
      setCapturing(null);
      force((n) => n + 1);
    };
    window.addEventListener("keydown", on, true);
    return () => window.removeEventListener("keydown", on, true);
  }, [capturing]);
  return (
    <section>
      <h3>Shortcuts</h3>
      <p className="muted">
        Click a key, then press the new one (letters, digits and symbols; Esc
        cancels). Arrow keys, Home/End, Delete and the ⌘/Ctrl shortcuts stay
        fixed.
      </p>
      <table className="set-table" data-testid="shortcut-table">
        <tbody>
          {REBINDABLE.map((r) => {
            const k = bindingOf(r.id);
            return (
              <tr key={r.id}>
                <td>{r.label}</td>
                <td>
                  <button
                    className={`btn kbd-btn${capturing === r.id ? " on" : ""}`}
                    onClick={() => setCapturing(r.id)}
                    data-testid={`bind-${r.id}`}
                  >
                    {capturing === r.id ? "Press a key…" : keyLabel(k)}
                  </button>
                </td>
                <td>
                  {k !== r.key && (
                    <button
                      className="btn"
                      onClick={() => {
                        setBinding(r.id, null);
                        force((n) => n + 1);
                      }}
                    >
                      Reset
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <button
        className="btn"
        onClick={() => {
          resetBindings();
          force((n) => n + 1);
        }}
      >
        Reset all to defaults
      </button>
    </section>
  );
}

export function Settings({ onError }: { onError: (e: AppError) => void }) {
  const [section, setSection] = useState<Section>("library");
  const [info, setInfo] = useState<AppInfo_Serialize | null>(null);
  const [catalog, setCatalog] = useState<Catalog>({ models: [], keys: [] });
  const [agent, setAgent] = useState<AgentStatus_Serialize | null>(null);
  const [agentKey, setAgentKey] = useState("");
  const [reports, setReports] = useState<CrashReport[]>([]);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [crash, setCrash] = useState(() => {
    try {
      return localStorage.getItem("ove.crashReports") === "on";
    } catch {
      return false;
    }
  });

  useEffect(() => {
    void commands.appInfo().then((r) => r.status === "ok" && setInfo(r.data));
    void loadCatalog().then(setCatalog);
    void commands
      .agentStatus()
      .then((r) => r.status === "ok" && setAgent(r.data));
    void commands.crashReports().then((r) => {
      setReports(r);
      if (r.some((x) => !x.seen)) void commands.markCrashReportsSeen();
    });
  }, []);

  const saveAgentKey = async () => {
    const r = await commands.agentSetKey(agentKey);
    if (r.status === "error") return onError(r.error);
    setAgentKey("");
    setAgent(r.data);
  };

  return (
    <div className="settings" data-testid="settings">
      <nav className="set-nav">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            className={`nav-item${section === s.id ? " active" : ""}`}
            onClick={() => setSection(s.id)}
            data-testid={`set-${s.id}`}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <div className="set-body">
        {section === "library" && (
          <section>
            <h3>Library</h3>
            <p className="muted">
              Projects are folders (bundles) in the library. Media you import
              stays where it is; the project links to it.
            </p>
            <div className="set-row">
              <span className="mono">{info?.libraryDir ?? "…"}</span>
              {info?.libraryDir && (
                <button
                  className="btn"
                  onClick={() => void commands.revealPath(info.libraryDir!)}
                >
                  Show in folder
                </button>
              )}
            </div>
            <p className="muted small">
              To use another folder, start the app with the{" "}
              <span className="mono">OVE_LIBRARY</span> environment variable set
              to it. The command line uses the same variable.
            </p>
          </section>
        )}
        {section === "generation" && (
          <section>
            <h3>Generation</h3>
            <p className="muted">
              Images, video, voice, music and sound effects are made with your
              own accounts. Each request shows its estimated cost and asks
              before spending; the provider bills you directly.
            </p>
            {catalog.keys.map((k) => (
              <KeyEntry
                key={k.provider}
                status={k}
                onSaved={setCatalog}
                onError={onError}
              />
            ))}
          </section>
        )}
        {section === "assistant" && (
          <section>
            <h3>Assistant</h3>
            <p className="muted">
              The assistant uses your Anthropic API key, kept in the system
              credential store and sent only to Anthropic.
            </p>
            <div className="gen-key">
              <div className="gen-key-head">
                <strong>Anthropic</strong>
                <span className={`badge${agent?.hasKey ? " ok" : ""}`}>
                  {agent?.hasKey ? "key saved" : "no key"}
                </span>
              </div>
              <div className="gen-key-row">
                <input
                  type="password"
                  placeholder="sk-ant-…"
                  value={agentKey}
                  onChange={(e) => setAgentKey(e.target.value)}
                  aria-label="Anthropic API key"
                />
                <button
                  className="btn"
                  disabled={!agentKey.trim()}
                  onClick={() => void saveAgentKey()}
                >
                  Save
                </button>
                {agent?.hasKey && (
                  <button
                    className="btn"
                    onClick={async () => {
                      const r = await commands.agentClearKey();
                      if (r.status === "error") onError(r.error);
                      else setAgent(r.data);
                    }}
                  >
                    Remove
                  </button>
                )}
              </div>
            </div>
          </section>
        )}
        {section === "agents" && (
          <section>
            <h3>Agents & command line</h3>
            <p className="muted">
              Claude Code, Codex and other agents edit your projects with the{" "}
              <span className="mono">ove</span> command line or its MCP server.
              While a project is open here, their edits appear in the editor at
              once and can be undone.
            </p>
            <h4>Give an agent the editing guide</h4>
            <Copyable text="ove skills install" />
            <h4>Connect the MCP server</h4>
            <Copyable text="claude mcp add openvideoeditor -- ove mcp" />
            <Copyable text="codex mcp add openvideoeditor -- ove mcp" />
            <h4>Check the setup</h4>
            <Copyable text="ove doctor" />
            <p className="muted small">
              <span className="mono">ove</span> is installed with the app (next
              to the app on macOS and Windows;{" "}
              <span className="mono">/usr/bin/ove</span> on Linux).{" "}
              <span className="mono">ove docs agents</span> prints the guide.
            </p>
          </section>
        )}
        {section === "shortcuts" && <ShortcutsSection />}
        {section === "privacy" && (
          <section>
            <h3>Privacy</h3>
            <ul className="set-list">
              <li>No usage data or analytics are collected.</li>
              <li>
                Projects and media stay on this computer. Files leave it only
                when you send them to a provider you set up (assistant,
                generation).
              </li>
              <li>
                API keys are kept in the system credential store, never in
                project files or logs.
              </li>
            </ul>
            <label className="gen-check">
              <input
                type="checkbox"
                checked={crash}
                onChange={(e) => {
                  setCrash(e.target.checked);
                  try {
                    localStorage.setItem(
                      "ove.crashReports",
                      e.target.checked ? "on" : "off",
                    );
                  } catch {
                    // not persisted
                  }
                }}
                data-testid="crash-opt-in"
              />{" "}
              Offer to attach the crash report when I send feedback after a
              crash
            </label>
            <p className="muted small">
              Crash reports and logs are written on this computer only:
            </p>
            <div className="set-row">
              <span className="mono">{info?.logDir ?? "…"}</span>
              {info?.logDir && (
                <button
                  className="btn"
                  onClick={() => void commands.revealPath(info.logDir!)}
                >
                  <Icon name="media" size={14} /> Show logs
                </button>
              )}
            </div>
            {reports.length > 0 && (
              <ul className="gen-jobs" data-testid="crash-list">
                {reports.slice(0, 8).map((r) => (
                  <li key={r.path} className="gen-job">
                    <div className="gen-job-text">
                      <span className="gen-job-prompt">
                        {r.summary || "Problem report"}
                      </span>
                      <span className="muted small">
                        {new Date(r.at ?? 0).toLocaleString()}
                      </span>
                    </div>
                    {crash && (
                      <button
                        className="btn"
                        onClick={async () => {
                          const t = await commands.readCrashReport(r.path);
                          if (t.status === "error") return onError(t.error);
                          void navigator.clipboard?.writeText(t.data);
                        }}
                      >
                        Copy
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}
        {section === "about" && (
          <section>
            <h3>About & updates</h3>
            <p>
              OpenVideoEditor {info?.version ?? ""} · {info?.os} · {info?.arch}
            </p>
            <p className="muted small">{info?.decodeCapabilities}</p>
            <div className="set-row">
              <button
                className="btn"
                disabled={checking}
                onClick={async () => {
                  setChecking(true);
                  const r = await commands.checkForUpdates();
                  setChecking(false);
                  if (r.status === "error") onError(r.error);
                  else setUpdate(r.data);
                }}
                data-testid="check-updates"
              >
                {checking ? "Checking…" : "Check for updates"}
              </button>
              {update?.available && update.url && (
                <button
                  className="btn primary"
                  onClick={() => void commands.openUrl(update.url!)}
                >
                  Download {update.latest}
                </button>
              )}
            </div>
            {update && (
              <p className="small" data-testid="update-result">
                {update.available
                  ? `Version ${update.latest} is available.`
                  : (update.message ??
                    `You have the latest version (${update.current}).`)}
              </p>
            )}
            {update?.notes && <p className="muted small">{update.notes}</p>}
          </section>
        )}
      </div>
    </div>
  );
}
