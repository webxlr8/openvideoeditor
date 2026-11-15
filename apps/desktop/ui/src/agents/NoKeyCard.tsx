// What the chat shows when no API key is saved. An assistant already installed on this computer comes first, and no
// key is asked for; the key form (the children) is one click away, and is shown at once when nothing is installed.
import { type ReactNode, useEffect, useState } from "react";

import type { AgentInfo } from "../bindings";
import { commands } from "../bridge";
import { type ErrorSink, logError, runCommand } from "../notify";
import { AgentsSection } from "./AgentsSection";

export function NoKeyCard({ children, onError }: { children: ReactNode; onError: ErrorSink }) {
  const [agents, setAgents] = useState<AgentInfo[] | null>(null);
  const [useKey, setUseKey] = useState(false);
  useEffect(() => {
    let live = true;
    runCommand(
      commands.detectAgents(),
      (data) => {
        if (live) setAgents(data);
      },
      (e) => {
        // Not being able to look leaves the key form, as before.
        logError(e);
        if (live) setAgents([]);
      },
    );
    return () => {
      live = false;
    };
  }, []);

  if (agents === null) return <p className="muted small chat-looking">Looking for assistants on this computer…</p>;
  const installed = agents.filter((a) => a.state !== "notInstalled");
  if (useKey || installed.length === 0) return <>{children}</>;
  return (
    <div className="key-card" data-testid="agents-card">
      <h3>Use an assistant you already have</h3>
      <p className="muted">
        Connect one, start it in its own window and ask it to edit this project. Every change shows up here as it is
        made, and can be undone.
      </p>
      <AgentsSection onError={onError} agents={installed} />
      <button className="btn" data-testid="use-api-key" onClick={() => setUseKey(true)}>
        Use an API key instead
      </button>
    </div>
  );
}
