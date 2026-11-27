import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { HistoryRow } from "./bindings";

const ROWS: HistoryRow[] = [
  {
    rev: 3,
    time: 1700000000000,
    source: "mcp",
    who: "MCP",
    summary: "source apply: updated 1 item",
    canRestore: true,
    client: "claude-code",
  },
  {
    rev: 2,
    time: 1699999990000,
    source: "gui",
    who: "You",
    summary: "move clip",
    canRestore: true,
    client: null,
  },
];

vi.mock("./bridge", () => ({
  commands: {
    projectHistory: () => Promise.resolve({ status: "ok", data: ROWS }),
    restoreBefore: () => Promise.resolve({ status: "ok", data: null }),
  },
}));

import { HistoryPanel } from "./HistoryPanel";

describe("HistoryPanel", () => {
  it("shows the client of a change made over MCP", async () => {
    render(
      <HistoryPanel rev={3} onClose={() => {}} onEdit={() => {}} onError={() => {}} />,
    );
    const rows = await screen.findAllByTestId("history-row");
    expect(rows[0].textContent).toMatch(/^MCPclaude-code/);
    expect(rows[1].textContent).toMatch(/^YouMove clip/);
    expect(screen.getAllByTestId("history-client")).toHaveLength(1);
  });
});
