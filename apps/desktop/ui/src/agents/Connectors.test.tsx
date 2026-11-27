import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { initialConnectors } from "./connectorsMock";

const mock = () => window.__oveMock!;

async function openAgents() {
  render(<App />);
  fireEvent.click(screen.getByTestId("nav-settings"));
  fireEvent.click(await screen.findByTestId("set-agents"));
  await screen.findByTestId("connector-claude-desktop");
}

describe("settings: desktop clients", () => {
  beforeEach(() => {
    mock().connectors = initialConnectors();
  });

  it("shows a client that is not installed with a link to get it and the manual configuration", async () => {
    await openAgents();
    expect(screen.getByTestId("connector-state-claude-desktop").textContent).toBe("not installed");
    expect(screen.queryByTestId("connector-connect-claude-desktop")).toBeNull();
    fireEvent.click(screen.getByTestId("connector-get-claude-desktop"));
    expect(mock().calls).toContain("openUrl:https://example.com/claude-desktop");
    const manual = screen.getByTestId<HTMLDetailsElement>("connector-manual-claude-desktop");
    expect(manual.open).toBe(true);
    expect(screen.getByTestId("connector-snippet-claude-desktop").textContent).toContain('"args": [\n        "mcp"');
    expect(manual.textContent).toContain("claude_desktop_config.json");
  });

  it("connects an installed Claude Desktop through its extension", async () => {
    mock().connectors.installed = ["claude-desktop"];
    await openAgents();
    expect(screen.getByTestId("connector-state-claude-desktop").textContent).toBe("installed, not connected");
    fireEvent.click(screen.getByTestId("connector-connect-claude-desktop"));
    expect((await screen.findByTestId("connector-tried-claude-desktop")).textContent).toContain(
      "Claude Desktop asks you to install the extension.",
    );
    fireEvent.click(screen.getByTestId("connector-check-claude-desktop"));
    await waitFor(() =>
      expect(screen.getByTestId("connector-state-claude-desktop").textContent).toBe("connected"),
    );
    expect(screen.getByTestId("connector-disconnect-claude-desktop")).toBeTruthy();
  });

  it("shows the manual steps and a copy button when the client refuses", async () => {
    mock().connectors.installed = ["claude-desktop"];
    mock().connectors.refuse = ["claude-desktop"];
    await openAgents();
    expect(screen.getByTestId<HTMLDetailsElement>("connector-manual-claude-desktop").open).toBe(false);
    fireEvent.click(screen.getByTestId("connector-connect-claude-desktop"));
    fireEvent.click(await screen.findByTestId("connector-check-claude-desktop"));
    expect((await screen.findByTestId("connector-message-claude-desktop")).textContent).toContain(
      "did not take the connection",
    );
    expect(screen.getByTestId<HTMLDetailsElement>("connector-manual-claude-desktop").open).toBe(true);
    const written: string[] = [];
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: (t: string) => (written.push(t), Promise.resolve()) },
      configurable: true,
    });
    fireEvent.click(screen.getByTestId("connector-copy-claude-desktop"));
    await waitFor(() => expect(screen.getByTestId("connector-copy-claude-desktop").textContent).toBe("Copied"));
    expect(written[0]).toContain("openvideoeditor");
    Reflect.deleteProperty(navigator, "clipboard");
  });

  it("opens Cursor's install link, and shows a connected Cursor with Disconnect", async () => {
    mock().connectors.installed = ["cursor"];
    await openAgents();
    fireEvent.click(screen.getByTestId("connector-connect-cursor"));
    expect((await screen.findByTestId("connector-tried-cursor")).textContent).toContain(
      "Cursor asks you to confirm the server.",
    );
    expect(mock().calls).toContain("connectorConnect:cursor");
    fireEvent.click(screen.getByTestId("connector-check-cursor"));
    await waitFor(() => expect(screen.getByTestId("connector-state-cursor").textContent).toBe("connected"));
    fireEvent.click(screen.getByTestId("connector-disconnect-cursor"));
    expect((await screen.findByTestId("connector-message-cursor")).textContent).toContain(
      "Removed the editor's server from C:/Users/you/.cursor/mcp.json",
    );
    await waitFor(() =>
      expect(screen.getByTestId("connector-state-cursor").textContent).toBe("installed, not connected"),
    );
  });
});
