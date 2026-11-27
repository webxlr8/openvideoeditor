import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TaskView_Serialize, TasksView_Serialize } from "../bindings";
import { TasksButton } from "./TasksPopover";

const mock = () => window.__oveMock!;

function task(id: number, over: Partial<TaskView_Serialize> = {}): TaskView_Serialize {
  return {
    id,
    kind: "skim",
    label: `Skim sheet: clip${id}.mov`,
    project: "/lib/Demo.ove",
    priority: "derivative",
    state: "running",
    pauseReason: null,
    progress: null,
    runningAlone: false,
    ...over,
  };
}

function show(view: TasksView_Serialize) {
  act(() => mock().setTasks(view));
}

async function openPopover() {
  fireEvent.click(await screen.findByTestId("tasks-button"));
  return screen.getByTestId("tasks-popover");
}

describe("background tasks popover", () => {
  it("shows no indicator while no background work exists", async () => {
    show({ tasks: [], paused: [], runAnyway: [] });
    const { container } = render(<TasksButton onError={vi.fn()} />);
    await waitFor(() => expect(mock().calls).toContain("listTasks"));
    expect(container.innerHTML).toBe("");
    expect(screen.queryByTestId("tasks-button")).toBeNull();
  });

  it("shows running tasks with progress, waiting tasks, and one running alone", async () => {
    show({ tasks: [], paused: [], runAnyway: [] });
    render(<TasksButton onError={vi.fn()} />);
    show({
      tasks: [
        task(1, { progress: 0.45 }),
        task(2, { kind: "proxy", label: "Proxy: a.mov", runningAlone: true, progress: 0.1 }),
        task(3, { state: "queued", label: "Thumbnail: b.mov" }),
      ],
      paused: [],
      runAnyway: [],
    });
    expect((await screen.findByTestId("tasks-button")).textContent).toContain("3 tasks");
    await openPopover();
    expect(screen.getByTestId("task-state-1").textContent).toBe("Running 45%");
    expect(screen.getByTestId("task-state-2").textContent).toBe("Running alone 10% (needs a lot of memory)");
    expect(screen.getByTestId("task-state-3").textContent).toBe("Waiting");
  });

  it("shows why work is paused and runs it anyway", async () => {
    show({ tasks: [], paused: [], runAnyway: [] });
    render(<TasksButton onError={vi.fn()} />);
    show({
      tasks: [task(4, { state: "paused", pauseReason: "battery", kind: "captions", priority: "analysis" })],
      paused: [{ reason: "battery", label: "on battery" }],
      runAnyway: [],
    });
    expect((await screen.findByTestId("tasks-button")).textContent).toContain("paused");
    await openPopover();
    expect(screen.getByTestId("tasks-reason-battery").textContent).toContain("Paused: on battery");
    expect(screen.getByTestId("task-state-4").textContent).toBe("Paused: on battery");
    fireEvent.click(screen.getByTestId("tasks-run-anyway-battery"));
    expect(mock().calls).toContain("runAnyway:battery:true");
    await waitFor(() => expect(screen.queryByTestId("tasks-reason-battery")).toBeNull());
    expect(screen.getByTestId("tasks-overridden-battery")).toBeTruthy();
  });

  it("cancels one task and leaves the others; the indicator goes when nothing is left", async () => {
    show({ tasks: [], paused: [], runAnyway: [] });
    render(<TasksButton onError={vi.fn()} />);
    show({
      tasks: [task(5), task(6, { priority: "export", kind: "export", label: "Export: final.mp4" })],
      paused: [],
      runAnyway: [],
    });
    await openPopover();
    // Export is cancelled from its own dialog.
    expect(screen.queryByTestId("task-cancel-6")).toBeNull();
    fireEvent.click(screen.getByTestId("task-cancel-5"));
    expect(mock().calls).toContain("cancelTask:5");
    await waitFor(() => expect(screen.queryByTestId("task-5")).toBeNull());
    expect(screen.getByTestId("task-6")).toBeTruthy();
    show({ tasks: [], paused: [], runAnyway: [] });
    await waitFor(() => expect(screen.queryByTestId("tasks-button")).toBeNull());
  });

  it("pauses and resumes all background work", async () => {
    show({ tasks: [], paused: [], runAnyway: [] });
    render(<TasksButton onError={vi.fn()} />);
    show({ tasks: [task(7)], paused: [], runAnyway: [] });
    await openPopover();
    fireEvent.click(screen.getByTestId("tasks-pause-all"));
    expect(mock().calls).toContain("pauseTasks:true");
    await waitFor(() => expect(screen.getByTestId("tasks-pause-all").textContent).toBe("Resume all"));
    // Paused by the person, the indicator stays even with nothing queued, so work can be resumed.
    show({ ...mock().tasks, tasks: [] });
    expect(await screen.findByTestId("tasks-button")).toBeTruthy();
    fireEvent.click(screen.getByTestId("tasks-pause-all"));
    expect(mock().calls).toContain("pauseTasks:false");
    await waitFor(() => expect(screen.queryByTestId("tasks-button")).toBeNull());
  });

  it("links the export queue, and shows while exports wait even with no other task", async () => {
    show({ tasks: [], paused: [], runAnyway: [] });
    const onOpen = vi.fn();
    const { rerender } = render(<TasksButton onError={vi.fn()} exports={{ count: 0, onOpen }} />);
    await waitFor(() => expect(mock().calls).toContain("listTasks"));
    expect(screen.queryByTestId("tasks-button")).toBeNull();
    rerender(<TasksButton onError={vi.fn()} exports={{ count: 2, onOpen }} />);
    await openPopover();
    expect(screen.getByTestId("tasks-exports").textContent).toContain("Exports (2)");
    fireEvent.click(screen.getByTestId("tasks-open-exports"));
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("tasks-popover")).toBeNull();
  });
});
