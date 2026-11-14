import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import type { ItemView_Serialize, ProjectView_Serialize } from "../bindings";
import { commands } from "../bridge";
import { STEPS, stepsShownBy } from "./checklist";
import { FIRST_RUN, SETTLED, applyChange } from "./state";

const mock = () => window.__oveMock!;
const welcome = () => screen.queryByTestId("welcome");
const step = (id: string) => screen.getByTestId(`checklist-step-${id}`).getAttribute("data-done");
const clips = () => screen.queryAllByTestId("clip");

/** The app on a first launch: no onboarding state yet. */
async function firstRun() {
  mock().onboarding = { ...FIRST_RUN };
  render(<App />);
  return screen.findByTestId("welcome");
}

/** The app with the welcome behind it and the checklist showing. */
async function withChecklist() {
  mock().onboarding = { ...FIRST_RUN, welcome: "finished" };
  render(<App />);
  await screen.findByTestId("checklist");
}

beforeEach(() => {
  mock().onboarding = { ...SETTLED };
  mock().calls.length = 0;
});

describe("welcome", () => {
  it("shows three screens on a first run, never asks for a key, and does not come back once finished", async () => {
    await firstRun();
    expect(screen.getByTestId("welcome-screen-what")).toBeTruthy();
    expect(screen.getByTestId("welcome-progress").textContent).toContain("1 of 3");
    fireEvent.click(screen.getByTestId("welcome-next"));

    // Pick an assistant: by hand is chosen until the person picks something else.
    expect(screen.getByTestId("welcome-screen-assistant")).toBeTruthy();
    expect(screen.getByTestId("assistant-option-none").getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByTestId("assistant-option-api-key"));
    expect(screen.getByTestId("assistant-option-api-key").getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByTestId("welcome-next"));

    // Privacy: what stays, what the assistant sees, and that nothing is sent without one.
    expect(screen.getByTestId("welcome-screen-privacy")).toBeTruthy();
    expect(screen.getByTestId("welcome-progress").textContent).toContain("3 of 3");
    const privacy = screen.getByTestId("welcome-privacy").textContent ?? "";
    expect(privacy).toContain("Media files stay on this computer.");
    expect(privacy).toMatch(/timeline, frames of your video, sound measurements and\s+transcripts go to that assistant's provider/);
    expect(privacy).toContain("Nothing is sent when no assistant is chosen.");
    expect(screen.queryByTestId("welcome-next")).toBeNull();

    // Back keeps the choice.
    fireEvent.click(screen.getByTestId("welcome-back"));
    expect(screen.getByTestId("assistant-option-api-key").getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByTestId("welcome-next"));
    fireEvent.click(screen.getByTestId("welcome-done"));

    await waitFor(() => expect(welcome()).toBeNull());
    expect(screen.getByTestId("home")).toBeTruthy();
    await waitFor(() => expect(mock().onboarding.welcome).toBe("finished"));
    expect(mock().onboarding.assistant).toBe("api-key");
    // The guided first edit is offered.
    expect(await screen.findByTestId("checklist")).toBeTruthy();
  });

  it("has no field to type a key into on any screen", async () => {
    const dialog = await firstRun();
    for (let i = 0; i < 3; i++) {
      expect(dialog.querySelectorAll("input, textarea, select").length).toBe(0);
      expect(dialog.textContent).not.toMatch(/paste|sk-ant|enter your key/i);
      if (i < 2) fireEvent.click(screen.getByTestId("welcome-next"));
    }
  });

  it.each([0, 1, 2])("Skip on screen %i goes to Home and the welcome does not return", async (screenIndex) => {
    await firstRun();
    for (let i = 0; i < screenIndex; i++) fireEvent.click(screen.getByTestId("welcome-next"));
    fireEvent.click(screen.getByTestId("welcome-skip"));
    await waitFor(() => expect(welcome()).toBeNull());
    expect(screen.getByTestId("home")).toBeTruthy();
    await waitFor(() => expect(mock().onboarding.welcome).toBe("skipped"));
    expect(mock().onboarding.assistant).toBeNull();
    // Skipping also keeps the checklist out of the way.
    expect(screen.queryByTestId("checklist")).toBeNull();

    // The next launch.
    cleanup();
    render(<App />);
    await waitFor(() => expect(screen.queryByTestId("onboarding-loading")).toBeNull());
    expect(welcome()).toBeNull();
  });

  it("is not shown when onboarding is over, and Escape skips it when it is", async () => {
    render(<App />);
    await waitFor(() => expect(screen.queryByTestId("onboarding-loading")).toBeNull());
    expect(welcome()).toBeNull();
    cleanup();

    await firstRun();
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(welcome()).toBeNull());
    expect(mock().onboarding.welcome).toBe("skipped");
  });

  it("keeps keys from reaching the window under it", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    await waitFor(() => expect(clips().length).toBe(1));
    fireEvent.click(screen.getByTestId("help"));
    fireEvent.click(screen.getByTestId("help-welcome"));
    await screen.findByTestId("welcome");
    // Delete would remove the selected clip in the editor; with the welcome open it does nothing.
    fireEvent.keyDown(document.body, { key: "Delete" });
    fireEvent.keyDown(screen.getByTestId("welcome-next"), { key: "Delete" });
    await act(async () => new Promise((r) => setTimeout(r, 30)));
    expect(clips().length).toBe(1);
    // Skipping a welcome shown again changes nothing: it was finished before.
    fireEvent.click(screen.getByTestId("welcome-skip"));
    await waitFor(() => expect(welcome()).toBeNull());
    expect(mock().onboarding.welcome).toBe("finished");
    expect(screen.getByTestId("editor")).toBeTruthy();
  });
});

describe("sample project", () => {
  it("opens from the welcome's last screen with a cut, a title, captions and a transition", async () => {
    await firstRun();
    fireEvent.click(screen.getByTestId("welcome-next"));
    fireEvent.click(screen.getByTestId("welcome-next"));
    fireEvent.click(screen.getByTestId("welcome-sample"));
    await screen.findByTestId("editor");
    expect(welcome()).toBeNull();
    await waitFor(() => expect(clips().length).toBe(5));
    const view = (await commands.projectView()) as { status: "ok"; data: ProjectView_Serialize };
    const items = view.data.items;
    const main = items.filter((i) => i.trackId === "t_main");
    expect(main.length).toBe(2);
    expect(main[1].transitionIn).toBeGreaterThan(0);
    expect(items.filter((i) => i.kind === "text").length).toBe(1);
    expect(items.filter((i) => i.kind === "caption").length).toBe(2);
    expect(mock().onboarding.welcome).toBe("finished");
    // What the sample already holds is not the person's first edit: nothing is ticked.
    await screen.findByTestId("checklist");
    for (const s of STEPS) expect(step(s.id)).toBe("false");
  });

  it("makes a second copy when opened again, from Home and from Help", async () => {
    const count = async () => {
      const r = await commands.listProjects();
      return r.status === "ok" ? r.data.length : -1;
    };
    const before = await count();
    render(<App />);
    fireEvent.click(screen.getByTestId("home-sample"));
    await screen.findByTestId("editor");
    await waitFor(() => expect(clips().length).toBe(5));
    // An edit in the first copy: a new name.
    fireEvent.click(screen.getByTestId("project-name"));
    const name = screen.getByLabelText("Project name");
    fireEvent.change(name, { target: { value: "Sample, edited" } });
    fireEvent.keyDown(name, { key: "Enter" });
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Sample, edited"));

    fireEvent.click(screen.getByTestId("help"));
    fireEvent.click(screen.getByTestId("help-sample"));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toMatch(/^Sample project/));
    expect(await count()).toBe(before + 2);
    fireEvent.click(screen.getByTestId("go-home"));
    await screen.findByTestId("home");
    await waitFor(() => expect(screen.getAllByTestId("project-card").length).toBe(before + 2));
    const names = screen.getAllByTestId("project-card").map((c) => c.textContent);
    expect(names.filter((n) => n?.includes("Sample, edited")).length).toBe(1);
  });
});

describe("guided first edit", () => {
  it("ticks Import, Cut and Title from what the project shows, without further action", async () => {
    await withChecklist();
    expect(screen.getByTestId("checklist-count").textContent).toBe("0 of 5");
    expect(screen.getByTestId("checklist-next").textContent).toContain("Add a video");

    // Import: a new project with one imported file.
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    await waitFor(() => expect(step("import")).toBe("true"));
    expect(step("cut")).toBe("false");
    expect(screen.getByTestId("checklist-next").textContent).toContain("Split");

    // Cut: split the clip at the playhead.
    await waitFor(() => expect(clips().length).toBe(1));
    fireEvent.pointerDown(clips()[0], { button: 0, clientX: 400 });
    fireEvent.pointerUp(window);
    await waitFor(() => expect(screen.getByTestId("split").hasAttribute("disabled")).toBe(false));
    fireEvent.click(screen.getByTestId("split"));
    await waitFor(() => expect(step("cut")).toBe("true"));

    // Title: one from the Text panel.
    fireEvent.click(screen.getByTestId("rail-text"));
    fireEvent.click(await screen.findByTestId("text-preset-headline"));
    await waitFor(() => expect(step("title")).toBe("true"));
    expect(screen.getByTestId("checklist-count").textContent).toBe("3 of 5");
    expect(mock().onboarding.stepsDone).toEqual(["import", "cut", "title"]);
    // Undoing the title does not untick it: the step was done.
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(clips().length).toBe(2));
    expect(step("title")).toBe("true");
  });

  it("ticks Captions when caption items appear and Export when an export finishes", async () => {
    await withChecklist();
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    await waitFor(() => expect(clips().length).toBe(1));
    // Captions added by any route (here: the bridge, as the assistant or the command line would).
    await act(async () => {
      await commands.addItems(
        JSON.stringify([
          { trackId: "t_captions", type: "caption", startTime: 0, endTime: 2, captionText: "Hello" },
        ]),
        null,
      );
    });
    await waitFor(() => expect(step("captions")).toBe("true"));

    // A cancelled export ticks nothing; a finished one does.
    expect(step("export")).toBe("false");
    await act(async () => {
      await commands.exportProject({ path: "/mock/exports/a.mp4", height: 720, fps: 30 });
      await commands.cancelExport();
    });
    expect(step("export")).toBe("false");
    await act(async () => {
      await commands.exportProject({ path: "/mock/exports/a.mp4", height: 720, fps: 30 });
    });
    await waitFor(() => expect(step("export")).toBe("true"), { timeout: 3000 });
  });

  it("closes, and Help → Getting started brings it back with the same steps ticked", async () => {
    mock().onboarding = { ...FIRST_RUN, welcome: "finished", stepsDone: ["import", "title"] };
    render(<App />);
    await screen.findByTestId("checklist");
    expect(step("import")).toBe("true");
    expect(step("title")).toBe("true");
    fireEvent.click(screen.getByTestId("checklist-close"));
    await waitFor(() => expect(screen.queryByTestId("checklist")).toBeNull());
    await waitFor(() => expect(mock().onboarding.checklistClosed).toBe(true));

    fireEvent.click(screen.getByTestId("help"));
    fireEvent.click(screen.getByTestId("help-getting-started"));
    await screen.findByTestId("checklist");
    expect(step("import")).toBe("true");
    expect(step("cut")).toBe("false");
    expect(step("title")).toBe("true");
    expect(mock().onboarding.checklistClosed).toBe(false);
  });

  it("all five done says so", async () => {
    mock().onboarding = {
      ...FIRST_RUN,
      welcome: "finished",
      stepsDone: ["import", "cut", "title", "captions", "export"],
    };
    render(<App />);
    await screen.findByTestId("checklist");
    expect(screen.getByTestId("checklist-count").textContent).toBe("5 of 5");
    expect(screen.getByTestId("checklist-next").textContent).toContain("All five done");
  });
});

describe("steps from project state", () => {
  const item = (id: string, kind: string, start: number, end: number, mediaId: string | null = null) => ({
    id,
    trackId: kind === "caption" ? "t_captions" : kind === "text" ? "t_text" : "t_main",
    kind,
    name: id,
    start,
    end,
    trimStart: 0,
    mediaId,
    pending: false,
    hidden: false,
    muted: false,
    text: null,
    transitionIn: 0,
    transitionOut: 0,
    speed: 1,
  });
  const view = (rev: number, items: ItemView_Serialize[], media = 1, id = "p1") =>
    ({
      id,
      rev,
      items,
      media: Array.from({ length: media }, (_, i) => ({ id: `m${i}` })),
    }) as unknown as ProjectView_Serialize;
  const clip = item("a", "video", 0, 10, "m0");

  it("opening a project ticks nothing, whatever it holds", () => {
    const full = view(5, [clip, item("t", "text", 0, 2), item("c", "caption", 0, 2)]);
    expect(stepsShownBy(null, full)).toEqual([]);
    expect(stepsShownBy(view(4, [], 0, "other"), full)).toEqual([]);
    expect(stepsShownBy(full, null)).toEqual([]);
    // The same revision again (a media check re-sending the view) is not an edit.
    expect(stepsShownBy(full, { ...full })).toEqual([]);
  });

  it("an import, a split, a title and captions each tick their step", () => {
    const empty = view(1, [], 0);
    const one = view(2, [clip]);
    expect(stepsShownBy(empty, one)).toEqual(["import"]);
    const split = view(3, [{ ...clip, end: 4 }, { ...item("a_b", "video", 4, 10, "m0"), trimStart: 4 }]);
    expect(stepsShownBy(one, split)).toEqual(["cut"]);
    expect(stepsShownBy(split, view(4, [...split.items, item("t", "text", 0, 3)]))).toEqual(["title"]);
    expect(stepsShownBy(split, view(4, [...split.items, item("c", "caption", 0, 3)]))).toEqual(["captions"]);
    // Several at once (an assistant turn that adds a title and captions).
    expect(
      stepsShownBy(one, view(5, [clip, item("t", "text", 0, 3), item("c", "caption", 0, 3)])),
    ).toEqual(["title", "captions"]);
  });

  it("a trim, a duplicate and a second import are not a cut", () => {
    const one = view(2, [clip]);
    expect(stepsShownBy(one, view(3, [{ ...clip, end: 6 }]))).toEqual([]);
    expect(stepsShownBy(one, view(3, [clip, item("copy", "video", 10, 20, "m0")]))).toEqual([]);
    expect(stepsShownBy(one, view(3, [clip, item("b", "video", 10, 20, "m1")], 2))).toEqual(["import"]);
    // Removing a title is not adding one.
    const titled = view(3, [clip, item("t", "text", 0, 3)]);
    expect(stepsShownBy(titled, view(4, [clip]))).toEqual([]);
  });

  it("changes follow the native rules", () => {
    const skipped = applyChange(FIRST_RUN, { kind: "skipWelcome" });
    expect(skipped).toMatchObject({ welcome: "skipped", checklistClosed: true });
    const finished = applyChange(FIRST_RUN, { kind: "finishWelcome", assistant: "claude" });
    expect(finished).toMatchObject({ welcome: "finished", assistant: "claude", checklistClosed: false });
    expect(applyChange(finished, { kind: "skipWelcome" })).toBe(finished);
    expect(applyChange(finished, { kind: "setAssistant", assistant: " " }).assistant).toBeNull();
    const stepped = applyChange(finished, { kind: "completeSteps", steps: ["title", "import", "title"] });
    expect(stepped.stepsDone).toEqual(["import", "title"]);
  });
});

describe("help", () => {
  it("on Home reaches getting started, the welcome, the sample and the shortcuts in one step each", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("help"));
    const menu = within(screen.getByTestId("help-menu"));
    expect(menu.getAllByRole("menuitem").map((b) => b.textContent)).toEqual([
      "Getting started",
      "Show the welcome again",
      "Try the sample project",
      "Keyboard shortcuts",
    ]);
    for (const b of menu.getAllByRole("menuitem")) expect(b.hasAttribute("disabled")).toBe(false);

    // Keyboard shortcuts: Settings, on its Shortcuts section.
    fireEvent.click(screen.getByTestId("help-shortcuts"));
    expect(screen.queryByTestId("help-menu")).toBeNull();
    await screen.findByTestId("settings");
    expect(screen.getByTestId("set-shortcuts").className).toContain("active");
    expect(screen.getByTestId("bind-split")).toBeTruthy();
    // Settings from the sidebar still opens on its first section.
    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => expect(screen.getByTestId("set-library").className).toContain("active"));

    // Show the welcome again: the earlier choice is still picked; finishing returns to where the person was.
    mock().onboarding = { ...SETTLED, assistant: "api-key" };
    cleanup();
    render(<App />);
    await waitFor(() => expect(screen.queryByTestId("onboarding-loading")).toBeNull());
    fireEvent.click(screen.getByTestId("help"));
    fireEvent.click(screen.getByTestId("help-welcome"));
    await screen.findByTestId("welcome");
    fireEvent.click(screen.getByTestId("welcome-next"));
    expect(screen.getByTestId("assistant-option-api-key").getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByTestId("assistant-option-none"));
    fireEvent.click(screen.getByTestId("welcome-next"));
    expect(screen.getByTestId("welcome-choice").textContent).toBe("You chose to edit by hand.");
    fireEvent.click(screen.getByTestId("welcome-done"));
    await waitFor(() => expect(welcome()).toBeNull());
    await waitFor(() => expect(mock().onboarding.assistant).toBeNull());
    expect(screen.getByTestId("home")).toBeTruthy();
  });

  it("in the editor lists the keyboard shortcuts and opens the sample", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    fireEvent.click(screen.getByTestId("help"));
    fireEvent.click(screen.getByTestId("help-shortcuts"));
    expect(await screen.findByTestId("shortcuts-overlay")).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("shortcuts-overlay")).toBeNull());

    fireEvent.click(screen.getByTestId("help"));
    fireEvent.click(screen.getByTestId("help-sample"));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toMatch(/^Sample project/));
    expect(mock().calls).toContain("createSampleProject");
  });
});
