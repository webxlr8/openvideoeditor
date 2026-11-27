import { Profiler } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { elapsed } from "./Reasoning";

beforeEach(() => {
  const agent = window.__oveMock!.agent;
  agent.hasKey = true;
  agent.stepMs = 5;
  agent.ask = false;
  try {
    localStorage.removeItem("ove.reasoningOpen");
  } catch {
    // no storage
  }
});

afterEach(() => {
  window.__oveMock!.agent.reasoning = undefined;
});

async function send(text: string, wrap?: (ui: React.ReactElement) => React.ReactElement) {
  render(wrap ? wrap(<App />) : <App />);
  fireEvent.click(screen.getByTestId("home-blank"));
  await screen.findByTestId("editor");
  const composer = await screen.findByTestId("composer");
  fireEvent.change(composer, { target: { value: text } });
  fireEvent.keyDown(composer, { key: "Enter" });
}

describe("reasoning", () => {
  it("formats the elapsed time", () => {
    expect(elapsed(400)).toBe("0s");
    expect(elapsed(4_600)).toBe("5s");
    expect(elapsed(75_000)).toBe("1m 15s");
  });

  it("streams into a collapsed block that expands, keeps updating, and remembers being open", async () => {
    window.__oveMock!.agent.stepMs = 20;
    window.__oveMock!.agent.reasoning =
      "The title should sit in the top third.\nThen check it does not cover the face. " + "More thought. ".repeat(30);
    await send("Add a title");
    const block = await screen.findByTestId("reasoning");
    // Collapsed: the first line and the time, not the text.
    expect(screen.queryByTestId("reasoning-body")).toBeNull();
    await waitFor(() => expect(block.textContent).toContain("The title should sit in the top third."));
    expect(block.textContent).not.toContain("does not cover the face");
    expect(screen.getByTestId("reasoning-time").textContent).toMatch(/^Thinking \d+s$/);
    fireEvent.click(screen.getByTestId("reasoning-toggle"));
    const body = screen.getByTestId("reasoning-body");
    const before = body.textContent.length;
    await waitFor(() => expect(screen.getByTestId("reasoning-body").textContent.length).toBeGreaterThan(before));
    await waitFor(() => expect(screen.getByTestId("reasoning-body").textContent).toContain("does not cover the face"));
    // The answer follows: the block stops and says how long it took.
    await waitFor(() => expect(screen.getByTestId("reasoning-time").textContent).toMatch(/^Thought for \d+s$/), { timeout: 5000 });
    fireEvent.click(screen.getByTestId("reasoning-toggle"));
    expect(screen.queryByTestId("reasoning-body")).toBeNull();
    fireEvent.click(screen.getByTestId("reasoning-toggle"));
    expect(localStorage.getItem("ove.reasoningOpen")).toBe("1");
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 5000 });
  });

  it("draws at most once per frame while streaming at 200 tokens a second", async () => {
    // 600 reasoning tokens, 4 every timer step (about 16 ms on Windows): about 200 tokens/s.
    window.__oveMock!.agent.stepMs = 15;
    window.__oveMock!.agent.reasoningPerStep = 4;
    window.__oveMock!.agent.reasoning = "token ".repeat(600);
    const commits: { at: number; ms: number }[] = [];
    let streaming = false;
    // Count the frames the chat asks for (a 16 ms timer stands in for the display).
    let frames = 0;
    const raf = globalThis.requestAnimationFrame;
    globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
      if (streaming) frames++;
      return setTimeout(() => cb(performance.now()), 16) as unknown as number;
    };
    await send("Think hard", (ui) => (
      <Profiler
        id="app"
        onRender={(_id, _phase, actual, _base, _start, commitTime) => {
          if (streaming) commits.push({ at: commitTime, ms: actual });
        }}
      >
        {ui}
      </Profiler>
    ));
    streaming = true;
    await screen.findByTestId("reasoning");
    await waitFor(() => expect(screen.getByTestId("reasoning-time").textContent).toMatch(/^Thought/), { timeout: 10000 });
    streaming = false;
    globalThis.requestAnimationFrame = raf;
    window.__oveMock!.agent.reasoningPerStep = undefined;
    const times = commits.map((c) => c.ms).sort((a, b) => a - b);
    const p95 = times[Math.floor(times.length * 0.95)] ?? 0;
    const span = (commits[commits.length - 1].at - commits[0].at) / 1000;
    const perSecond = commits.length / Math.max(span, 0.001);
    console.info(
      `reasoning at ${(600 / span).toFixed(0)} tok/s: ${frames} reasoning frames for 600 tokens, ${commits.length} commits in ${span.toFixed(2)} s (${perSecond.toFixed(0)}/s), render p95 ${p95.toFixed(2)} ms, max ${times[times.length - 1].toFixed(2)} ms`,
    );
    // Coalesced: one draw per frame, not per token, and each well inside a 16 ms frame.
    expect(frames).toBeLessThan(600 / 2);
    expect(p95).toBeLessThan(16);
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 5000 });
  }, 20000);
});
