import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { App } from "../App";
import type { AgentInfo } from "../bindings";
import { AgentsSection } from "./AgentsSection";
import { AssistantPicker } from "./AssistantPicker";

const agent = (id: string, name: string, state: AgentInfo["state"], runs = false): AgentInfo => ({
  id,
  name,
  vendor: `${name} Inc`,
  state,
  version: state === "ready" ? "1.2.3" : null,
  getIt: `https://example.com/${id}`,
  connectRuns: runs,
  connectInstructions: `${id} mcp add openvideoeditor -- ove mcp`,
});

describe("AssistantPicker", () => {
  it("lists every agent with an icon and its state, and only an installed one can be chosen", () => {
    const onChange = vi.fn();
    render(
      <AssistantPicker
        value={null}
        onChange={onChange}
        agents={[agent("claude", "Claude", "ready", true), agent("codex", "Codex", "found"), agent("grok", "Grok", "notInstalled")]}
      />,
    );
    expect(screen.getByTestId("assistant-claude").textContent).toContain("Ready · 1.2.3");
    expect(screen.getByTestId("assistant-codex").textContent).toContain("Installed");
    expect(screen.getByTestId("assistant-grok").textContent).toContain("Not installed");
    for (const id of ["claude", "codex", "grok", "api-key"]) expect(screen.getByTestId(`agent-icon-${id}`)).toBeTruthy();
    // The icons are told apart.
    const letters = ["claude", "codex", "grok"].map((id) => screen.getByTestId(`agent-icon-${id}`).textContent);
    expect(new Set(letters).size).toBe(3);

    expect(screen.getByTestId<HTMLButtonElement>("assistant-grok").disabled).toBe(true);
    fireEvent.click(screen.getByTestId("assistant-claude"));
    expect(onChange).toHaveBeenLastCalledWith("claude");
    fireEvent.click(screen.getByTestId("assistant-api-key"));
    expect(onChange).toHaveBeenLastCalledWith("api-key");
    // "I'll edit by hand" is the choice when nothing else is.
    expect(screen.getByTestId("assistant-none").getAttribute("aria-checked")).toBe("true");
    // Nothing here asks for a key.
    expect(document.querySelector("input")).toBeNull();
  });

  it("asks the app which agents are installed when it is not told", async () => {
    window.__oveMock!.localAgents = ["codex"];
    render(<AssistantPicker value="codex" onChange={() => undefined} />);
    const codex = await screen.findByTestId("assistant-codex");
    expect(codex.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByTestId<HTMLButtonElement>("assistant-claude").disabled).toBe(true);
    expect(screen.getAllByRole("radio").length).toBe(8);
  });
});

describe("AgentsSection", () => {
  it("connects with a button where the command is checked, and shows the command to copy elsewhere", async () => {
    window.__oveMock!.localAgents = ["claude", "codex"];
    render(<AgentsSection onError={() => undefined} />);
    fireEvent.click(await screen.findByTestId("agent-connect-claude"));
    await waitFor(() => expect(screen.getByTestId("agent-result-claude").textContent).toContain("Claude now has the editing tools"));
    expect(window.__oveMock!.calls).toContain("connectAgent:claude");
    // Codex is installed, but its command has not been checked: no button, the command to copy.
    expect(screen.queryByTestId("agent-connect-codex")).toBeNull();
    expect(screen.getByTestId("agent-instructions-codex").textContent).toBe("codex mcp add openvideoeditor -- ove mcp");
    // Not installed: where to get it.
    expect(screen.getByTestId("agent-get-grok")).toBeTruthy();
    expect(screen.queryByTestId("agent-connect-grok")).toBeNull();
  });
});

describe("the chat without a key", () => {
  const openChat = async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    fireEvent.click(screen.getByTestId("toggle-chat"));
  };

  it("offers an installed assistant first and does not ask for a key", async () => {
    window.__oveMock!.agent.hasKey = false;
    window.__oveMock!.localAgents = ["claude"];
    await openChat();
    const card = await screen.findByTestId("agents-card");
    expect(card.textContent).toContain("Use an assistant you already have");
    expect(screen.queryByTestId("key-card")).toBeNull();
    expect(screen.queryByTestId("key-input")).toBeNull();
    // Only what is installed is offered here.
    expect(screen.getByTestId("agent-claude")).toBeTruthy();
    expect(screen.queryByTestId("agent-grok")).toBeNull();
    // The key is one click away.
    fireEvent.click(screen.getByTestId("use-api-key"));
    expect(await screen.findByTestId("key-input")).toBeTruthy();
  });

  it("goes straight to the key when no assistant is installed", async () => {
    window.__oveMock!.agent.hasKey = false;
    window.__oveMock!.localAgents = [];
    await openChat();
    expect(await screen.findByTestId("key-card")).toBeTruthy();
    expect(screen.queryByTestId("agents-card")).toBeNull();
  });
});
