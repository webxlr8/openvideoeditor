import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import type { FeedbackDraft_Serialize } from "../bindings";

function draft(id: string, summary: string): FeedbackDraft_Serialize {
  return {
    id,
    createdAt: 1_700_000_000_000,
    category: "bug",
    severity: "medium",
    summary,
    from: "stand-in agent",
    recentTools: ["print_timeline", "apply_timeline"],
    lastError: "edit.anchor",
    edited: false,
    report: `[bug · medium] ${summary}\n`,
  };
}

function stubClipboard() {
  const writeText = vi.fn(() => Promise.resolve());
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  return writeText;
}

afterEach(() => {
  window.__oveMock!.feedbackDrafts = [];
  Reflect.deleteProperty(navigator, "clipboard");
});

async function openQueue() {
  render(<App />);
  fireEvent.click(await screen.findByTestId("help"));
  fireEvent.click(screen.getByTestId("help-feedback"));
  return screen.findByTestId("feedback-queue");
}

describe("Help → Feedback", () => {
  it("says when there are no reports", async () => {
    await openQueue();
    expect(await screen.findByTestId("feedback-queue-empty")).toBeTruthy();
  });

  it("lists agents' reports to edit, copy and delete; nothing is sent", async () => {
    window.__oveMock!.feedbackDrafts = [
      draft("fb_2", "Export stops at 99% on a long project"),
      draft("fb_1", "The receipt does not name the track"),
    ];
    const writeText = stubClipboard();
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    await openQueue();
    const rows = await screen.findAllByTestId("feedback-draft");
    expect(rows.map((r) => r.getAttribute("data-id"))).toEqual(["fb_2", "fb_1"]);
    const first = within(rows[0]);
    expect(rows[0].textContent).toContain("from stand-in agent");
    expect(rows[0].textContent).toContain("Recent tools: print_timeline, apply_timeline");
    expect(rows[0].textContent).toContain("Last error: edit.anchor");

    // Edit and save; copying waits until the change is saved.
    fireEvent.change(first.getByTestId("feedback-draft-text"), {
      target: { value: "Export stops at 99% when the project is longer than an hour" },
    });
    expect(first.getByTestId<HTMLButtonElement>("feedback-draft-copy").disabled).toBe(true);
    fireEvent.click(first.getByTestId("feedback-draft-save"));
    await waitFor(() => expect(rows[0].textContent).toContain("edited"));
    expect(window.__oveMock!.feedbackDrafts[0].summary).toBe(
      "Export stops at 99% when the project is longer than an hour",
    );
    fireEvent.click(first.getByTestId("feedback-draft-copy"));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(writeText.mock.calls[0]).toEqual([
      "[bug · medium] Export stops at 99% when the project is longer than an hour\n",
    ]);

    // Delete the other one.
    fireEvent.click(within(rows[1]).getByTestId("feedback-draft-delete"));
    await waitFor(() => expect(screen.getAllByTestId("feedback-draft")).toHaveLength(1));
    expect(window.__oveMock!.feedbackDrafts.map((d) => d.id)).toEqual(["fb_2"]);
    // No send button, no network.
    expect(screen.queryByText(/^Send$/)).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });
});
