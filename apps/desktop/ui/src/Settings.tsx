// Settings (change 13): library, generation provider keys, assistant key, agents and the command line, shortcuts,
// privacy. Keys go to the system credential store through the core; nothing here is stored in plain files.

import { useEffect, useState } from "react";
import type {
  AgentLimits_Serialize,
  AgentStatus_Serialize,
  AppError,
  AppInfo_Serialize,
  CrashReport,
  KeyCheck,
  UpdateInfo_Serialize as UpdateInfo,
} from "./bindings";
import { AgentsSection } from "./agents/AgentsSection";
import { commands } from "./bridge";
import { AssistantDataList } from "./ChatPanel";
import { copyText } from "./clipboard";
import { type Catalog, KeyEntry, loadCatalog } from "./Generate";
import { Icon } from "./icons";
import { fire, logError, runCommand } from "./notify";
import {
  REBINDABLE,
  RESERVED,
  bindingOf,
  customBindings,
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
          fire(
            copyText(text).then(() => {
              setDone(true);
              setTimeout(() => setDone(false), 1500);
            }),
          );
        }}
      >
        {done ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

/** A dollar amount typed in a box: empty is "no cap"; anything else must be a positive number. */
export function parseCap(text: string): number | null | "invalid" {
  const t = text.trim().replace(/^\$/, "");
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) && n > 0 ? n : "invalid";
}

function LimitsForm({
  limits,
  onSaved,
  onError,
}: {
  limits: AgentLimits_Serialize;
  onSaved: (s: AgentStatus_Serialize) => void;
  onError: (e: AppError) => void;
}) {
  const money = (v: number | null) => (v == null ? "" : v.toFixed(2));
  const [turnCap, setTurnCap] = useState(money(limits.turnCapUsd));
  const [monthCap, setMonthCap] = useState(money(limits.projectMonthCapUsd));
  const [requests, setRequests] = useState(String(limits.maxRequests));
  const [minutes, setMinutes] = useState(String(limits.maxMinutes));
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const save = () => {
    const t = parseCap(turnCap);
    const m = parseCap(monthCap);
    const r = Number(requests);
    const mins = Number(minutes);
    if (t === "invalid" || m === "invalid")
      return setProblem(
        "A spend cap must be an amount in dollars above $0, or empty for no cap.",
      );
    if (!Number.isInteger(r) || r < 1 || !Number.isInteger(mins) || mins < 1)
      return setProblem(
        "Max requests and minutes per turn must be whole numbers of at least 1.",
      );
    setProblem(null);
    runCommand(
      commands.agentSetLimits({
        turnCapUsd: t,
        projectMonthCapUsd: m,
        maxRequests: r,
        maxMinutes: mins,
      }),
      (s) => {
        setSaved(true);
        onSaved(s);
      },
      (e) => {
        setProblem(e.message);
        onError(e);
      },
    );
  };
  const field = (
    label: string,
    hint: string,
    value: string,
    set: (v: string) => void,
    id: string,
    placeholder?: string,
  ) => (
    <>
      <label htmlFor={id}>
        {label}
        <br />
        <span className="muted small">{hint}</span>
      </label>
      <input
        id={id}
        value={value}
        placeholder={placeholder}
        inputMode="decimal"
        data-testid={id}
        onChange={(e) => {
          set(e.target.value);
          setSaved(false);
        }}
      />
    </>
  );
  return (
    <div data-testid="assistant-limits">
      <div className="limits-grid">
        {field(
          "Spend cap per turn ($)",
          "The assistant stops when one turn has spent this much.",
          turnCap,
          setTurnCap,
          "limit-turn-cap",
          "no cap",
        )}
        {field(
          "Spend cap per project per month ($)",
          "No new turns in this project once its chats have spent this much this calendar month.",
          monthCap,
          setMonthCap,
          "limit-month-cap",
          "no cap",
        )}
        {field(
          "Max requests per turn",
          "Each step of a turn (reading, editing, checking) is one request.",
          requests,
          setRequests,
          "limit-requests",
        )}
        {field(
          "Max minutes per turn",
          "How long one turn may keep working.",
          minutes,
          setMinutes,
          "limit-minutes",
        )}
      </div>
      {problem && <p className="key-error">{problem}</p>}
      <button className="btn" onClick={save} data-testid="limits-save">
        {saved ? "Saved" : "Save limits"}
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
        disabled={Object.keys(customBindings()).length === 0}
        title={
          Object.keys(customBindings()).length === 0
            ? "Every key is already the default"
            : undefined
        }
        data-testid="bind-reset-all"
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
  const [keyCheck, setKeyCheck] = useState<KeyCheck | "checking" | null>(
    null,
  );
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
    runCommand(commands.appInfo(), setInfo, onError);
    fire(loadCatalog().then(setCatalog), onError);
    runCommand(commands.agentStatus(), setAgent, onError);
    fire(
      commands.crashReports().then((r) => {
        setReports(r);
        if (r.some((x) => !x.seen))
          runCommand(commands.markCrashReportsSeen(), undefined, logError);
      }),
      onError,
    );
  }, [onError]);

  /** Tries the saved key with a real (free) request. */
  const testAgentKey = () => {
    setKeyCheck("checking");
    runCommand(commands.agentCheckKey(), setKeyCheck, (e) => {
      setKeyCheck(null);
      onError(e);
    });
  };

  const saveAgentKey = async () => {
    const r = await commands.agentSetKey(agentKey);
    if (r.status === "error") return onError(r.error);
    setAgentKey("");
    setAgent(r.data);
    testAgentKey();
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
                  onClick={() =>
                    runCommand(
                      commands.revealPath(info.libraryDir!),
                      undefined,
                      onError,
                    )
                  }
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
            <p className="muted small" data-testid="generation-data">
              Generation sends the prompt and any reference image you attach
              to the provider you pick, and nothing else.
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
              credential store.
            </p>
            <AssistantDataList />
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
                  onClick={() => fire(saveAgentKey(), onError)}
                >
                  Save
                </button>
                {agent?.hasKey && (
                  <button
                    className="btn"
                    disabled={keyCheck === "checking"}
                    onClick={testAgentKey}
                    data-testid="agent-key-test"
                  >
                    Test
                  </button>
                )}
                {agent?.hasKey && (
                  <button
                    className="btn"
                    onClick={() => {
                      setKeyCheck(null);
                      runCommand(commands.agentClearKey(), setAgent, onError);
                    }}
                  >
                    Remove
                  </button>
                )}
              </div>
              {keyCheck && (
                <p
                  className={
                    keyCheck === "checking"
                      ? "muted small"
                      : keyCheck.ok
                        ? "key-ok small"
                        : "key-error small"
                  }
                  role="status"
                  data-testid="agent-key-check"
                >
                  {keyCheck === "checking"
                    ? "Checking the key…"
                    : keyCheck.message}
                </p>
              )}
            </div>
            <h4>Spending and limits</h4>
            <p className="muted small">
              Costs are estimates from the token counts the API reports;
              Anthropic bills you directly. Limits are checked between the
              assistant's requests, so a turn can end slightly over its cap.
            </p>
            {agent && (
              <LimitsForm
                limits={agent.limits}
                onSaved={setAgent}
                onError={onError}
              />
            )}
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
            <h4>Assistants on this computer</h4>
            <p className="muted small">
              Connect gives an installed assistant the editing guide and the
              editing tools, through the assistant's own command. The app
              never sees an assistant's sign-in.
            </p>
            <AgentsSection onError={onError} />
            <h4>By hand: give an agent the editing guide</h4>
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
            <ul className="set-list" data-testid="privacy-list">
              <li>
                The assistant sends the timeline as text (item names and
                times, text and caption words, media file names), the chat and
                the results of its tools to Anthropic. It never sends media
                files or video frames.
              </li>
              <li>
                Generation sends the prompt and any reference image you
                attach to the provider you pick.
              </li>
              <li>
                Nothing else leaves this computer: projects and media stay
                here, and there is no telemetry or analytics. (Check for
                updates asks for the latest version number only when you
                click it.)
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
                  onClick={() =>
                    runCommand(
                      commands.revealPath(info.logDir!),
                      undefined,
                      onError,
                    )
                  }
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
                        onClick={() =>
                          runCommand(
                            commands.readCrashReport(r.path),
                            (text) => fire(copyText(text), onError),
                            onError,
                          )
                        }
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
                onClick={() => {
                  setChecking(true);
                  fire(
                    commands
                      .checkForUpdates()
                      .then((r) => {
                        if (r.status === "error") onError(r.error);
                        else setUpdate(r.data);
                      })
                      .finally(() => setChecking(false)),
                    onError,
                  );
                }}
                data-testid="check-updates"
              >
                {checking ? "Checking…" : "Check for updates"}
              </button>
              {update?.available && update.url && (
                <button
                  className="btn primary"
                  onClick={() =>
                    runCommand(
                      commands.openUrl(update.url!),
                      undefined,
                      onError,
                    )
                  }
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
