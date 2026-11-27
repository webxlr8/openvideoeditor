// Settings → Agents: desktop clients connected in one step. Claude Desktop installs the editor as an extension;
// Cursor opens an install link. Each row says whether the client is installed and connected, and always offers the
// manual configuration to copy, which is also the way in when the client refuses or nothing handles the link.
import { useCallback, useEffect, useState } from "react";

import type { ConnectorInfo_Serialize as ConnectorInfo } from "../bindings";
import { commands } from "../bridge";
import { copyText } from "../clipboard";
import { type ErrorSink, fire, runCommand } from "../notify";

function stateText(c: ConnectorInfo): string {
  if (!c.installed) return "not installed";
  return c.connected ? "connected" : "installed, not connected";
}

export function ConnectorsSection({ onError }: { onError: ErrorSink }) {
  const [list, setList] = useState<ConnectorInfo[] | null>(null);
  /** Clients Connect was clicked for: the row then shows what happened and offers Check again. */
  const [tried, setTried] = useState<Record<string, string>>({});
  /** Clients still not connected after Check again: the manual steps open. */
  const [manual, setManual] = useState<string[]>([]);
  const [message, setMessage] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<string | null>(null);

  const look = useCallback(
    (after?: (l: ConnectorInfo[]) => void) =>
      runCommand(
        commands.connectors(),
        (l) => {
          setList(l);
          after?.(l);
        },
        onError,
      ),
    [onError],
  );
  useEffect(() => look(), [look]);

  const connect = (c: ConnectorInfo) =>
    runCommand(
      commands.connectorConnect(c.id),
      (r) => setTried((t) => ({ ...t, [c.id]: r.detail })),
      (e) => {
        setManual((m) => [...m, c.id]);
        setMessage((m) => ({ ...m, [c.id]: e.message }));
      },
    );
  const checkAgain = (c: ConnectorInfo) =>
    look((l) => {
      const now = l.find((x) => x.id === c.id);
      if (now && !now.connected) {
        setManual((m) => (m.includes(c.id) ? m : [...m, c.id]));
        setMessage((m) => ({
          ...m,
          [c.id]: `${c.name} did not take the connection. Add the server by hand: copy this into ${now.configFile ?? "its configuration file"}, then restart ${c.name}.`,
        }));
      } else {
        setMessage((m) => ({ ...m, [c.id]: `${c.name} has the editing tools.` }));
      }
    });
  const disconnect = (c: ConnectorInfo) =>
    runCommand(
      commands.connectorDisconnect(c.id),
      (removed) => {
        setMessage((m) => ({
          ...m,
          [c.id]: removed
            ? `Removed the editor's server from ${c.configFile ?? "the configuration"} (a copy of the old file was kept).`
            : c.kind === "extension"
              ? "Remove the OpenVideoEditor extension in Claude Desktop's settings, under Extensions."
              : "The configuration had no entry for the editor.",
        }));
        look();
      },
      onError,
    );
  const copy = (c: ConnectorInfo) =>
    fire(
      copyText(c.snippet).then(() => setCopied(c.id)),
      onError,
    );

  return (
    <div data-testid="connectors-section">
      {list === null && <p className="muted small">Looking for desktop apps…</p>}
      <div className="agent-lines">
        {(list ?? []).map((c) => (
          <div className="agent-line" key={c.id} data-testid={`connector-${c.id}`}>
            <div className="head">
              <span className="names">
                <strong>{c.name}</strong>{" "}
                <span className="muted small" data-testid={`connector-state-${c.id}`}>
                  {stateText(c)}
                </span>
              </span>
              {!c.installed && (
                <button
                  className="btn"
                  data-testid={`connector-get-${c.id}`}
                  onClick={() => runCommand(commands.openUrl(c.getIt), () => {}, onError)}
                >
                  Get {c.name}
                </button>
              )}
              {c.installed && !c.connected && (
                <button className="btn primary" data-testid={`connector-connect-${c.id}`} onClick={() => connect(c)}>
                  Connect
                </button>
              )}
              {c.connected && (
                <button className="btn" data-testid={`connector-disconnect-${c.id}`} onClick={() => disconnect(c)}>
                  Disconnect
                </button>
              )}
            </div>
            {tried[c.id] && !c.connected && (
              <p className="small" data-testid={`connector-tried-${c.id}`}>
                {tried[c.id]} Then{" "}
                <button className="btn" data-testid={`connector-check-${c.id}`} onClick={() => checkAgain(c)}>
                  Check again
                </button>
              </p>
            )}
            {message[c.id] && (
              <p className="small" data-testid={`connector-message-${c.id}`}>
                {message[c.id]}
              </p>
            )}
            <details open={!c.installed || manual.includes(c.id)} data-testid={`connector-manual-${c.id}`}>
              <summary className="small">Manual configuration</summary>
              <p className="muted small">
                Add this to <span className="mono">{c.configFile ?? "the configuration file"}</span>
                {c.kind === "extension" ? " (Claude Desktop → Settings → Developer → Edit Config)" : ""}, then restart{" "}
                {c.name}.
              </p>
              <pre className="set-code mono" data-testid={`connector-snippet-${c.id}`}>
                {c.snippet}
              </pre>
              <button className="btn" data-testid={`connector-copy-${c.id}`} onClick={() => copy(c)}>
                {copied === c.id ? "Copied" : "Copy"}
              </button>
            </details>
          </div>
        ))}
      </div>
    </div>
  );
}
