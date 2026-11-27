import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";
import { CachesSection } from "./CachesSection";

const mock = () => window.__oveMock!;

describe("settings: storage of derived data", () => {
  it("lists each kind with its size across projects when none is open", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("nav-settings"));
    fireEvent.click(await screen.findByTestId("set-storage"));
    const rows = await screen.findByTestId("caches-list");
    await waitFor(() => expect(rows.children.length).toBe(5));
    expect(screen.getByTestId("cache-waveforms").textContent).toContain("Audio peak cache");
    expect(screen.getByTestId("cache-total-skim").textContent).toBe("9.0 MB");
    // Without an open project there is nothing to clear here.
    expect(screen.queryByTestId("cache-clear-skim")).toBeNull();
    expect(screen.queryByTestId("use-proxies")).toBeNull();
  });

  it("clears one kind of the open project: its tasks first, never media or the document", async () => {
    const opened = await commands.createProject("Clear test", null);
    expect(opened.status).toBe("ok");
    const before = await commands.projectView();
    act(() =>
      mock().setTasks({
        tasks: [
          {
            id: 9,
            kind: "skim",
            label: "Skim sheet: a.mov",
            project: null,
            priority: "derivative",
            state: "running",
            pauseReason: null,
            progress: 0.5,
            runningAlone: false,
          },
        ],
        paused: [],
        runAnyway: [],
      }),
    );
    render(<CachesSection onError={vi.fn()} />);
    expect((await screen.findByTestId("cache-size-skim")).textContent).toBe("2.4 MB");
    fireEvent.click(screen.getByTestId("cache-clear-skim"));
    expect(mock().calls).toContain("clearCache:skim");
    const message = await screen.findByTestId("caches-message");
    expect(message.textContent).toBe("Skim sheets cleared: 2.4 MB freed. 1 running task was cancelled first.");
    expect(screen.getByTestId("cache-size-skim").textContent).toBe("0.0 MB");
    expect(screen.getByTestId("cache-total-skim").textContent).toBe("6.6 MB");
    expect(mock().tasks.tasks).toEqual([]);
    // Other kinds, the media and the document are as they were.
    expect(screen.getByTestId("cache-size-waveforms").textContent).toBe("0.3 MB");
    const after = await commands.projectView();
    expect(after).toEqual(before);
    // Nothing left: Clear is off for that kind.
    expect(screen.getByTestId<HTMLButtonElement>("cache-clear-skim").disabled).toBe(true);
  });

  it("turns the project's Use proxies setting on and off", async () => {
    await commands.createProject("Proxy test", null);
    render(<CachesSection onError={vi.fn()} />);
    const box = await screen.findByTestId<HTMLInputElement>("use-proxies");
    expect(box.checked).toBe(false);
    fireEvent.click(box);
    await waitFor(() => expect(screen.getByTestId<HTMLInputElement>("use-proxies").checked).toBe(true));
    const v = await commands.projectView();
    expect(v.status === "ok" && v.data?.useProxies).toBe(true);
    fireEvent.click(screen.getByTestId("use-proxies"));
    await waitFor(() => expect(screen.getByTestId<HTMLInputElement>("use-proxies").checked).toBe(false));
  });
});
