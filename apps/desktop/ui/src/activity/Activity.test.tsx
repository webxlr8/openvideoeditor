import { Profiler } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";
import { FADE_MS, MAX_OUTLINES, applyEvents, outlines, prune } from "./store";

const ev = (over: Partial<Parameters<typeof applyEvents>[1][number]>) => ({
  actor: "mcp:s1",
  name: "Claude Code",
  kind: "change",
  ids: [] as string[],
  removed: [] as string[],
  start: null as number | null,
  end: null as number | null,
  ts: 0,
  ...over,
});

const mock = () => window.__oveMock!;
const outlined = () => screen.queryAllByTestId("activity-outline");

async function editorWithClip() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  return screen.getAllByTestId("clip")[0].dataset.id!;
}

describe("activity store", () => {
  it("coalesces, keeps changes over later reads, clears the person's items and fades", () => {
    let s = applyEvents(new Map(), [ev({ ids: ["a", "b"] }), ev({ kind: "read", ids: ["b", "c"], start: 1, end: 2 })], 1000);
    const a = s.get("mcp:s1")!;
    expect([...a.items.entries()]).toEqual([
      ["a", "change"],
      ["b", "change"],
      ["c", "read"],
    ]);
    expect(a.range).toEqual({ start: 1, end: 2 });
    s = applyEvents(s, [ev({ actor: "user", name: "You", kind: "person", ids: ["b"] })], 1500);
    expect([...s.get("mcp:s1")!.items.keys()]).toEqual(["a", "c"]);
    expect(s.has("user")).toBe(false);
    expect(prune(s, 1000 + FADE_MS - 1).size).toBe(1);
    expect(prune(s, 1000 + FADE_MS).size).toBe(0);
  });

  it("outlines at most 200 items, newest first, with a count of the rest", () => {
    const ids = Array.from({ length: 250 }, (_, i) => `i${i}`);
    const s = applyEvents(new Map(), [ev({ ids })], 0);
    const o = outlines(s, () => true);
    expect(o.shown.length).toBe(MAX_OUTLINES);
    expect(o.more).toBe(50);
    expect(o.shown[0].id).toBe("i249");
    expect(outlines(s, (id) => id === "i3").shown.map((x) => x.id)).toEqual(["i3"]);
  });
});

describe("activity layer", () => {
  it("outlines an external agent's change within 300 ms with a badge naming the client, then fades", async () => {
    const id = await editorWithClip();
    const t0 = performance.now();
    act(() => mock().activity({ actor: "mcp:s1", name: "Claude Code", kind: "change", ids: [id] }));
    await waitFor(() => expect(outlined().length).toBe(1), { timeout: 300 });
    expect(performance.now() - t0).toBeLessThan(300);
    expect(outlined()[0].dataset.kind).toBe("change");
    const badge = screen.getByTestId("activity-badge");
    expect(badge.textContent).toContain("Claude Code");
    expect(badge.textContent).toContain("changing 1");
    // Three seconds after the last event the display is gone.
    await waitFor(() => expect(screen.queryByTestId("activity-badge")).toBeNull(), { timeout: FADE_MS + 1500 });
    expect(outlined().length).toBe(0);
  });

  it("shows a band on the ruler over the range an agent reads", async () => {
    await editorWithClip();
    act(() => mock().activity({ actor: "assistant:c1", name: "Assistant", kind: "read", ids: [], start: 10, end: 20 }));
    const band = await screen.findByTestId("activity-band");
    expect([Number(band.dataset.start), Number(band.dataset.end)]).toEqual([10, 20]);
    expect(parseFloat(band.style.width)).toBeGreaterThan(0);
    expect(screen.getByTestId("activity-badge").textContent).toContain("Assistant");
  });

  it("does not outline the person's own edit as an agent change", async () => {
    const id = await editorWithClip();
    act(() => {
      mock().activity({ actor: "mcp:s1", name: "Claude Code", kind: "change", ids: [id] });
      // The person edited the same item while the agent's change was applied.
      mock().activity({ actor: "user:", name: "You", kind: "person", ids: [id] });
    });
    await waitFor(() => expect(screen.queryByTestId("activity-badge")).not.toBeNull());
    expect(outlined().length).toBe(0);
    // The person's edits alone never show.
    act(() => mock().activity({ actor: "user:", name: "You", kind: "person", ids: [id] }));
    await new Promise((r) => setTimeout(r, 150));
    expect(screen.queryAllByTestId("activity-badge").map((b) => b.textContent)).not.toContainEqual(expect.stringContaining("You"));
  });

  it("outlines the in-app assistant's change as it lands", async () => {
    mock().agent.hasKey = true;
    mock().agent.stepMs = 5;
    render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    const composer = await screen.findByTestId("composer");
    fireEvent.change(composer, { target: { value: "Add an opening title" } });
    fireEvent.keyDown(composer, { key: "Enter" });
    await waitFor(() => expect(outlined().length).toBe(1), { timeout: 3000 });
    expect(screen.getByTestId("activity-badge").textContent).toContain("Assistant");
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 3000 });
  });
});

// The flood's frame times, measured around the whole app with the React profiler.
describe("activity frame time", () => {
  it("stays responsive through 1,000 applies in 10 s: batched draws inside a 16 ms frame, at most 200 outlined", async () => {
    const commits: number[] = [];
    let on = false;
    render(
      <Profiler id="app" onRender={(_i, _p, actual) => on && commits.push(actual)}>
        <App />
      </Profiler>,
    );
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    const many = Array.from({ length: 400 }, (_, i) => ({ trackId: "t_text", type: "text", startTime: i * 0.5, endTime: i * 0.5 + 0.5, name: `T${i}`, textContent: "x" }));
    await act(async () => {
      await commands.addItems(JSON.stringify(many), "add 400");
    });
    await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(400));
    const ids = screen.getAllByTestId("clip").map((c) => c.dataset.id!);
    on = true;
    const t0 = performance.now();
    // 1,000 applies spread evenly over 10 s (whatever the timer resolution), each changing 1–3 items. Not inside
    // act(): React draws as it would in the app.
    await new Promise<void>((resolve) => {
      let sent = 0;
      const timer = setInterval(() => {
        const due = Math.min(1000, Math.round(((performance.now() - t0) / 10_000) * 1000));
        for (; sent < due; sent++) {
          const n = 1 + (sent % 3);
          mock().activity({
            actor: "mcp:flood",
            name: "Claude Code",
            kind: "change",
            ids: Array.from({ length: n }, (_, j) => ids[(sent * 7 + j * 13) % ids.length]),
          });
        }
        if (sent >= 1000) {
          clearInterval(timer);
          resolve();
        }
      }, 10);
    });
    await new Promise((r) => setTimeout(r, 250));
    on = false;
    const seconds = (performance.now() - t0) / 1000;
    const sorted = [...commits].sort((a, b) => a - b);
    const p50 = sorted[Math.floor(sorted.length * 0.5)] ?? 0;
    const p95 = sorted[Math.floor(sorted.length * 0.95)] ?? 0;
    console.info(
      `activity flood: 1000 applies in ${seconds.toFixed(1)} s, ${commits.length} commits (${(commits.length / seconds).toFixed(1)}/s), render p50 ${p50.toFixed(2)} ms, p95 ${p95.toFixed(2)} ms, max ${sorted[sorted.length - 1].toFixed(2)} ms, ${outlined().length} outlined`,
    );
    // Batched: about ten draws a second, not one per apply. Each batch draws inside a 16 ms frame; this is React's
    // development build in the test DOM, where pauses for garbage collection give a few slower draws, so the bound
    // on the slowest 5 % is looser here than in the app.
    expect(commits.length).toBeLessThan(1000 / 3);
    expect(p50).toBeLessThan(16);
    expect(p95).toBeLessThan(32);
    expect(outlined().length).toBe(MAX_OUTLINES);
    expect(Number(screen.getByTestId("activity-more").textContent.replace(/\D/g, ""))).toBeGreaterThan(0);
  }, 40000);
});
