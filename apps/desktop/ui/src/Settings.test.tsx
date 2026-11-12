import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import { parseCap } from "./Settings";

async function openSettings(section: string) {
  render(<App />);
  fireEvent.click(screen.getByTestId("nav-settings"));
  fireEvent.click(await screen.findByTestId(`set-${section}`));
}

beforeEach(() => {
  window.__oveMock!.agent.hasKey = true;
  window.__oveMock!.agent.keyWorks = true;
});

describe("settings", () => {
  it("reads spend caps as dollars, empty meaning no cap", () => {
    expect(parseCap("")).toBeNull();
    expect(parseCap(" $2.50 ")).toBe(2.5);
    expect(parseCap("0")).toBe("invalid");
    expect(parseCap("lots")).toBe("invalid");
  });

  it("edits the assistant's spend caps and turn limits", async () => {
    await openSettings("assistant");
    const turnCap = await screen.findByTestId<HTMLInputElement>("limit-turn-cap");
    expect(turnCap.value).toBe("2.00");
    expect(screen.getByTestId<HTMLInputElement>("limit-month-cap").value).toBe("");
    expect(screen.getByTestId<HTMLInputElement>("limit-requests").value).toBe("30");
    expect(screen.getByTestId<HTMLInputElement>("limit-minutes").value).toBe("15");
    fireEvent.change(screen.getByTestId("limit-month-cap"), { target: { value: "abc" } });
    fireEvent.click(screen.getByTestId("limits-save"));
    expect(screen.getByTestId("assistant-limits").textContent).toContain("must be an amount in dollars");
    fireEvent.change(screen.getByTestId("limit-month-cap"), { target: { value: "25" } });
    fireEvent.change(turnCap, { target: { value: "" } });
    fireEvent.change(screen.getByTestId("limit-requests"), { target: { value: "50" } });
    fireEvent.click(screen.getByTestId("limits-save"));
    await waitFor(() => expect(screen.getByTestId("limits-save").textContent).toBe("Saved"));
    const s = await commands.agentStatus();
    expect(s.status === "ok" && s.data.limits).toEqual({
      turnCapUsd: null,
      projectMonthCapUsd: 25,
      maxRequests: 50,
      maxMinutes: 15,
    });
    // Back to the defaults for the other tests.
    await commands.agentSetLimits({ turnCapUsd: 2, projectMonthCapUsd: null, maxRequests: 30, maxMinutes: 15 });
  });

  it("tests the assistant key and says exactly what is sent", async () => {
    await openSettings("assistant");
    expect((await screen.findByTestId("assistant-data")).textContent).toContain(
      "the timeline as text (item names and times, text and caption words, media file names)",
    );
    window.__oveMock!.agent.keyWorks = false;
    fireEvent.click(screen.getByTestId("agent-key-test"));
    expect((await screen.findByTestId("agent-key-check")).textContent).toContain("rejected");
    window.__oveMock!.agent.keyWorks = true;
    fireEvent.click(screen.getByTestId("agent-key-test"));
    await waitFor(() => expect(screen.getByTestId("agent-key-check").textContent).toBe("Key works."));
  });

  it("privacy lists what leaves the computer and nothing vaguer", async () => {
    await openSettings("privacy");
    const list = (await screen.findByTestId("privacy-list")).textContent ?? "";
    expect(list).toContain("never sends media files or video frames");
    expect(list).toContain("prompt and any reference image you attach to the provider you pick");
    expect(list).toContain("no telemetry");
  });
});
