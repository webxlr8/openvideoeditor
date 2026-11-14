// Choosing the assistant: the supported agents found on this computer, each with its icon and whether it is ready,
// plus an API key and editing by hand. Never asks for a key.
import { useEffect, useState } from "react";

import type { AgentInfo } from "../bindings";
import { commands } from "../bridge";
import { logError, runCommand } from "../notify";
import { AgentIcon } from "./AgentIcon";

/** `null` is "I'll edit by hand"; `"api-key"` is the built-in assistant on the person's own key. */
export type AssistantChoice = string | null;

/** The version number out of what the program printed ("2.1.325 (Claude Code)" → "2.1.325"). */
function versionNumber(printed: string): string {
  return /\d+(?:\.\d+)+/.exec(printed)?.[0] ?? printed;
}

export function stateText(a: AgentInfo): string {
  if (a.state === "ready") return a.version ? `Ready · ${versionNumber(a.version)}` : "Ready";
  if (a.state === "found") return "Installed";
  return "Not installed";
}

export function AssistantPicker({
  value,
  onChange,
  agents: given,
  allowKey = true,
  allowByHand = true,
}: {
  value: AssistantChoice;
  onChange: (v: AssistantChoice) => void;
  /** For tests and callers that already asked; otherwise the picker asks the app. */
  agents?: AgentInfo[];
  allowKey?: boolean;
  allowByHand?: boolean;
}) {
  const [found, setFound] = useState<AgentInfo[] | null>(given ?? null);
  useEffect(() => {
    if (given) return;
    let live = true;
    runCommand(
      commands.detectAgents(),
      (data) => {
        if (live) setFound(data);
      },
      (e) => {
        // Not being able to look is not the person's problem: the key and by-hand choices remain.
        logError(e);
        if (live) setFound([]);
      },
    );
    return () => {
      live = false;
    };
  }, [given]);

  // Installed assistants first, each group in the list's own order.
  const all = given ?? found;
  const agents = all && [...all.filter((a) => a.state !== "notInstalled"), ...all.filter((a) => a.state === "notInstalled")];
  return (
    <div className="assistant-picker" role="radiogroup" aria-label="Assistant" data-testid="assistant-picker">
      {agents === null && <p className="muted small">Looking for assistants on this computer…</p>}
      {(agents ?? []).map((a) => (
        <button
          key={a.id}
          type="button"
          role="radio"
          className="assistant-row"
          data-testid={`assistant-${a.id}`}
          aria-checked={value === a.id}
          disabled={a.state === "notInstalled"}
          onClick={() => onChange(a.id)}
        >
          <AgentIcon id={a.id} name={a.name} />
          <span className="names">
            <span>{a.name}</span>
            <small>{a.vendor}</small>
          </span>
          <span className={`state${a.state === "ready" ? " ready" : ""}`}>{stateText(a)}</span>
        </button>
      ))}
      {allowKey && (
        <button
          type="button"
          role="radio"
          className="assistant-row"
          data-testid="assistant-api-key"
          aria-checked={value === "api-key"}
          onClick={() => onChange("api-key")}
        >
          <AgentIcon id="api-key" name="API key" />
          <span className="names">
            <span>Built-in assistant</span>
            <small>Uses your own API key, added later in Settings</small>
          </span>
        </button>
      )}
      {allowByHand && (
        <button
          type="button"
          role="radio"
          className="assistant-row"
          data-testid="assistant-none"
          aria-checked={value === null}
          onClick={() => onChange(null)}
        >
          <span className="names">
            <span>I'll edit by hand</span>
            <small>No assistant; nothing is sent anywhere</small>
          </span>
        </button>
      )}
    </div>
  );
}
