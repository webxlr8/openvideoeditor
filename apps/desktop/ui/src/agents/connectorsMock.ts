// The browser stand-in for Settings → Agents → desktop clients (Claude Desktop, Cursor).
import type { AppError, ConnectorInfo_Serialize, ConnectorOpened } from "../bindings";

type Ok<T> = { status: "ok"; data: T };
type Err = { status: "error"; error: AppError };
type Res<T> = Promise<Ok<T> | Err>;

export type MockConnectors = {
  /** Clients installed on this computer. */
  installed: string[];
  /** Clients whose configuration has the editor's server. */
  connected: string[];
  /** Clients that do not take the connection when Connect opens it (they refuse the extension, or nothing handles the
   * link). */
  refuse: string[];
};

export function initialConnectors(): MockConnectors {
  return { installed: [], connected: [], refuse: [] };
}

const OVE = "C:/Program Files/OpenVideoEditor/ove.exe";
const SNIPPET = JSON.stringify({ mcpServers: { openvideoeditor: { command: OVE, args: ["mcp"] } } }, null, 2);

export function createConnectorsMock(
  s: () => MockConnectors,
  ok: <T>(d: T) => Promise<Ok<T>>,
  fail: (code: AppError["code"], message: string) => Promise<Err>,
  track: (name: string) => void,
) {
  const rows: [string, string, string, string][] = [
    ["claude-desktop", "Claude Desktop", "extension", "C:/Users/you/AppData/Roaming/Claude/claude_desktop_config.json"],
    ["cursor", "Cursor", "link", "C:/Users/you/.cursor/mcp.json"],
  ];
  return {
    connectors: (): Res<ConnectorInfo_Serialize[]> =>
      ok(
        rows.map(([id, name, kind, configFile]) => ({
          id,
          name,
          kind,
          installed: s().installed.includes(id),
          connected: s().connected.includes(id),
          getIt: `https://example.com/${id}`,
          configFile,
          snippet: SNIPPET,
        })),
      ),
    connectorConnect: (id: string): Res<ConnectorOpened> => {
      track(`connectorConnect:${id}`);
      if (!s().installed.includes(id)) return fail("not_found", `${id} is not installed`);
      if (!s().refuse.includes(id)) s().connected.push(id);
      return ok(
        id === "cursor"
          ? { opened: "cursor://anysphere.cursor-deeplink/mcp/install?name=openvideoeditor&config=e30%3D", detail: "Cursor asks you to confirm the server." }
          : { opened: "C:/Users/you/AppData/Local/cache/openvideoeditor-win32.mcpb", detail: "Claude Desktop asks you to install the extension." },
      );
    },
    connectorDisconnect: (id: string): Res<boolean> => {
      track(`connectorDisconnect:${id}`);
      const was = s().connected.includes(id);
      s().connected = s().connected.filter((c) => c !== id);
      return ok(was);
    },
  };
}
