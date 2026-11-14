// Settings → Agents: the supported agents on this computer and Connect, which gives an agent used from its own
// window the app's editing tools. Connect runs the agent's own command where we have checked it; for the others it
// shows what to do.
import { useCallback, useEffect, useState } from "react";

import type { AgentConnect, AgentInfo } from "../bindings";
import { commands } from "../bridge";
import { copyText } from "../clipboard";
import { type ErrorSink, fire, runCommand } from "../notify";
import { AgentIcon } from "./AgentIcon";
import { stateText } from "./AssistantPicker";

function outcomeText(name: string, r: AgentConnect): string {
  if (r.status === "connected") return `${name} now has the editing tools. Start it and ask it to edit your project.`;
  if (r.status === "alreadyConnected") return `${name} already has the editing tools.`;
  if (r.status === "manual") return "";
  return `${name} did not accept it: ${r.detail || "no reason given"}`;
}

export function AgentsSection({
  onError,
  agents: given,
}: {
  onError: ErrorSink;
  /** Show exactly these (the caller already looked); otherwise the section looks itself and offers to look again. */
  agents?: AgentInfo[];
}) {
  const [found, setAgents] = useState<AgentInfo[] | null>(given ?? null);
  const agents = given ?? found;
  const [busy, setBusy] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, AgentConnect>>({});
  const [copied, setCopied] = useState<string | null>(null);

  const look = useCallback(() => {
    setAgents(null);
    runCommand(commands.detectAgents(), setAgents, onError);
  }, [onError]);
  useEffect(() => {
    if (!given) runCommand(commands.detectAgents(), setAgents, onError);
  }, [given, onError]);

  const connect = (a: AgentInfo) => {
    setBusy(a.id);
    fire(
      commands
        .connectAgent(a.id)
        .then((r) => {
          if (r.status === "ok") setResults((prev) => ({ ...prev, [a.id]: r.data }));
          else onError(r.error);
        })
        .finally(() => setBusy(null)),
      onError,
    );
  };

  return (
    <div data-testid="agents-section">
      {agents === null && <p className="muted small">Looking for assistants on this computer…</p>}
      <div className="agent-lines">
        {(agents ?? []).map((a) => {
          const result = results[a.id];
          const installed = a.state !== "notInstalled";
          return (
            <div className="agent-line" key={a.id} data-testid={`agent-${a.id}`}>
              <div className="head">
                <AgentIcon id={a.id} name={a.name} />
                <span className="names">
                  <span>{a.name}</span>
                  <small>{a.vendor}</small>
                </span>
                <span className={`state${a.state === "ready" ? " ready" : ""}`}>{stateText(a)}</span>
                {installed && a.connectRuns && (
                  <button
                    className="btn"
                    data-testid={`agent-connect-${a.id}`}
                    disabled={busy !== null}
                    onClick={() => connect(a)}
                  >
                    {busy === a.id ? "Connecting…" : "Connect"}
                  </button>
                )}
                {!installed && (
                  <button
                    className="btn"
                    data-testid={`agent-get-${a.id}`}
                    onClick={() => runCommand(commands.openUrl(a.getIt), undefined, onError)}
                  >
                    Get it
                  </button>
                )}
              </div>
              {installed && !a.connectRuns && (
                <div className="set-code">
                  <code data-testid={`agent-instructions-${a.id}`}>{a.connectInstructions}</code>
                  <button
                    className="btn"
                    onClick={() =>
                      fire(
                        copyText(a.connectInstructions).then(() => {
                          setCopied(a.id);
                          setTimeout(() => setCopied(null), 1500);
                        }),
                        onError,
                      )
                    }
                  >
                    {copied === a.id ? "Copied" : "Copy"}
                  </button>
                </div>
              )}
              {result && result.status !== "manual" && (
                <p className={`note${result.status === "failed" ? " bad" : ""}`} data-testid={`agent-result-${a.id}`}>
                  {outcomeText(a.name, result)}
                  {result.status === "failed" ? `\nYou can run it yourself:\n${result.instructions}` : ""}
                </p>
              )}
            </div>
          );
        })}
      </div>
      {!given && (
        <button className="btn" data-testid="agents-look-again" onClick={look} disabled={agents === null}>
          Look again
        </button>
      )}
    </div>
  );
}
