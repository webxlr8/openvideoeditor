import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type { HistoryRow } from "./bindings";
import { commands } from "./bridge";
import { groupHistory } from "./HistoryPanel";

const CLAUDE = { kind: "mcp", session: "s1", client: "claude-code", name: "Claude Code" };

/** The sample project with its clip, the person's and Claude Code's edits interleaved, and History open. */
async function historyWithAgentEdits(personAlsoEditsTheSameField: boolean) {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  const id = screen.getAllByTestId("clip")[0].dataset.id!;
  const mock = window.__oveMock!;
  await act(async () => {
    mock.agentEdit(CLAUDE, id, "name", "Agent name");
    await new Promise((r) => setTimeout(r, 0));
    mock.personEdit(id, "muted", true);
    await new Promise((r) => setTimeout(r, 0));
    mock.agentEdit(CLAUDE, id, "hidden", true);
    await new Promise((r) => setTimeout(r, 0));
    if (personAlsoEditsTheSameField) mock.personEdit(id, "name", "My name");
    await new Promise((r) => setTimeout(r, 0));
  });
  fireEvent.click(screen.getByTestId("history"));
  await screen.findAllByTestId("history-group");
  return id;
}

const clip = async (id: string) => {
  const r = await commands.projectView();
  return r.status === "ok" ? r.data?.items.find((i) => i.id === id) : undefined;
};

describe("history by agent", () => {
  it("groups consecutive rows by actor and offers undo only for agents", () => {
    const row = (rev: number, who: string, actorKind: string | null, session: string | null, actorName: string | null, turn: string | null = null): HistoryRow => ({
      rev,
      time: 0,
      source: actorKind ?? "gui",
      who,
      summary: "x",
      canRestore: true,
      client: null,
      actorKind,
      session,
      turn,
      actorName,
    });
    const groups = groupHistory([
      row(5, "MCP", "mcp", "s1", "Claude Code"),
      row(4, "MCP", "mcp", "s1", "Claude Code"),
      row(3, "You", null, null, null),
      row(2, "Assistant", "assistant", "c1", "Assistant", "u_1"),
      row(1, "Assistant", "assistant", "c1", "Assistant", "u_1"),
    ]);
    expect(groups.map((g) => [g.name, g.rows.length])).toEqual([
      ["Claude Code", 2],
      ["You", 1],
      ["Assistant", 2],
    ]);
    expect(groups[0].agent).toEqual({ kind: "mcp", session: "s1", turn: null });
    expect(groups[1].agent).toBeNull();
    // An assistant group made in one turn undoes that turn.
    expect(groups[2].agent).toEqual({ kind: "assistant", session: "c1", turn: "u_1" });
  });

  it("undoes only Claude Code's changes and keeps the person's", async () => {
    const id = await historyWithAgentEdits(false);
    const groups = screen.getAllByTestId("history-group");
    expect(groups.map((g) => g.querySelector(".history-group-name")?.textContent)).toEqual(["Claude Code", "You", "Claude Code", "You"]);
    // Only agent groups offer the undo.
    expect(within(groups[1]).queryByTestId("history-undo-agent")).toBeNull();
    fireEvent.click(within(groups[0]).getByTestId("history-undo-agent"));
    await waitFor(async () => expect((await clip(id))?.hidden).toBe(false));
    const c = await clip(id);
    expect(c?.name).not.toBe("Agent name");
    expect(c?.muted).toBe(true);
    await waitFor(() => expect(screen.getAllByTestId("history-row")[0].textContent).toMatch(/^YouUndo the session \(2 changes\)/));
    expect(screen.queryByTestId("history-conflicts")).toBeNull();
  });

  it("refuses when the person changed the same field later, listing it and changing nothing", async () => {
    const id = await historyWithAgentEdits(true);
    const before = await clip(id);
    const claude = screen.getAllByTestId("history-group").find((g) => g.textContent?.includes("Claude Code"))!;
    fireEvent.click(within(claude).getByTestId("history-undo-agent"));
    const conflicts = await screen.findByTestId("history-conflicts");
    expect(conflicts.textContent).toContain("Nothing was changed");
    expect(conflicts.textContent).toContain(`${id} · name`);
    expect(conflicts.textContent).toContain("edit name");
    expect(await clip(id)).toEqual(before);
  });
});
