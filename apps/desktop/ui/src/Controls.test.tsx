// Controls that used to do nothing (or something other than their label) on Home, in menus, in the shortcut list
// and in Settings.

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ProjectSummary_Serialize } from "./bindings";
import { App } from "./App";
import { CHIPS, Home, matchProjects } from "./Home";
import { setBinding } from "./keys";
import { COMING_LATER, ContextMenu } from "./Menu";
import { Shortcuts } from "./Shortcuts";

const summary = (name: string): ProjectSummary_Serialize => ({
  bundle: `/lib/${name}.ove`,
  id: name,
  name,
  itemCount: 1,
  aspectRatio: "16:9",
  durationSeconds: 5,
  updatedAt: 0,
  thumbnail: null,
  damaged: null,
});

const home = (projects: ProjectSummary_Serialize[] = []) =>
  render(
    <Home
      projects={projects}
      onOpenFile={() => {}}
      onBlank={() => {}}
      onOpenProject={() => {}}
      onPrompt={() => {}}
    />,
  );

describe("Home", () => {
  it("fills the prompt from a chip without starting anything", () => {
    const onPrompt = vi.fn();
    render(
      <Home
        projects={[]}
        onOpenFile={() => {}}
        onBlank={() => {}}
        onOpenProject={() => {}}
        onPrompt={onPrompt}
      />,
    );
    fireEvent.click(screen.getByTestId("chip-Travel diary"));
    const prompt = screen.getByTestId<HTMLTextAreaElement>("home-prompt");
    expect(prompt.value).toBe(
      CHIPS.find((c) => c.label === "Travel diary")!.prompt,
    );
    expect(document.activeElement).toBe(prompt);
    expect(onPrompt).not.toHaveBeenCalled();
    fireEvent.change(prompt, { target: { value: `${prompt.value}Lisbon` } });
    fireEvent.click(screen.getByTestId("home-send"));
    expect(onPrompt).toHaveBeenCalledWith(
      "Make a travel diary from my clips: Lisbon",
    );
  });

  it("searches projects by name, and ⌘K / Ctrl+K focuses the search", () => {
    home([summary("Lisbon trip"), summary("Launch video"), summary("Interview")]);
    const search = screen.getByTestId<HTMLInputElement>("home-search");
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(document.activeElement).toBe(search);
    fireEvent.change(search, { target: { value: "l" } });
    expect(screen.getByTestId("projects-page")).toBeTruthy();
    expect(
      screen.getAllByTestId("project-card").map((c) => c.textContent),
    ).toEqual([
      expect.stringContaining("Lisbon trip"),
      expect.stringContaining("Launch video"),
    ]);
    fireEvent.change(search, { target: { value: "nothing like it" } });
    expect(screen.queryAllByTestId("project-card")).toHaveLength(0);
    expect(screen.getByTestId("search-empty").textContent).toContain(
      "nothing like it",
    );
    fireEvent.keyDown(search, { key: "Escape" });
    expect(search.value).toBe("");
    expect(screen.getAllByTestId("project-card")).toHaveLength(3);
  });

  it("matches names regardless of case and surrounding spaces", () => {
    const all = [summary("Harbour Talks"), summary("Product tour")];
    expect(matchProjects(all, "  harbour ").map((p) => p.name)).toEqual([
      "Harbour Talks",
    ]);
    expect(matchProjects(all, "")).toBe(all);
  });

  it("explains every disabled control and has no stubs left", () => {
    home();
    for (const b of screen.getAllByRole<HTMLButtonElement>("button"))
      if (b.disabled) expect(b.title).toBeTruthy();
    // The old "Ideas (later)" stub is gone: the chips are the ideas.
    expect(screen.queryByTitle(/later/i)).toBeNull();
    expect(screen.getByText("Local profile").closest(".profile")?.getAttribute("title"))
      .toContain("No account");
  });
});

describe("menus", () => {
  it("shows items without an action as unavailable, with their tooltip", () => {
    const onSelect = vi.fn();
    render(
      <ContextMenu
        at={{ x: 10, y: 10 }}
        onClose={() => {}}
        items={[
          { id: "fit", label: "Zoom to fit", onSelect },
          { id: "snapshot", label: "Copy frame", title: COMING_LATER },
        ]}
      />,
    );
    const later = screen.getByTestId<HTMLButtonElement>("menu-snapshot");
    expect(later.disabled).toBe(true);
    expect(later.title).toBe("Coming in a later version");
    expect(screen.getByTestId<HTMLButtonElement>("menu-fit").disabled).toBe(false);
  });

  it("marks the viewer's Copy frame as coming later", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    fireEvent.contextMenu(screen.getByTestId("viewer-rect"));
    const item = screen.getByTestId<HTMLButtonElement>("menu-snapshot");
    expect(item.textContent).toBe("Copy frame");
    expect(item.disabled).toBe(true);
    expect(item.title).toBe(COMING_LATER);
    // So is attaching files in the assistant.
    const attach = screen.getByTestId<HTMLButtonElement>("attach");
    expect(attach.disabled).toBe(true);
    expect(attach.title).toBe(COMING_LATER);
  });

  it("disables the empty conversation list entry and New conversation in an empty chat", async () => {
    window.__oveMock!.agent.hasKey = true;
    render(<App />);
    fireEvent.click(screen.getByTestId("home-open"));
    await screen.findByTestId("editor");
    if (!screen.queryByTestId("chat")) fireEvent.click(screen.getByTestId("toggle-chat"));
    await waitFor(() =>
      expect(screen.getByTestId<HTMLButtonElement>("new-chat").disabled).toBe(true),
    );
    fireEvent.click(screen.getByTestId("chat-history-button"));
    const none = screen.queryByTestId<HTMLButtonElement>("chat-history-item-none");
    if (none) expect(none.disabled).toBe(true);
  });
});

describe("shortcut list", () => {
  it("shows the keys as they are bound now", () => {
    setBinding("split", "x");
    setBinding("zoomIn", "]");
    render(<Shortcuts onClose={() => {}} />);
    const row = (what: string) =>
      screen.getByText(what).closest(".shortcut-row")?.querySelector("kbd")
        ?.textContent;
    expect(row("Split the selected clip at the playhead")).toBe("X");
    expect(row("Zoom in / out")).toBe("] / −");
    expect(row("Duplicate selection")).toBe("D");
    expect(row("Reverse, pause, forward (repeat for 2×)")).toBe("J / K / L");
    setBinding("split", null);
    setBinding("zoomIn", null);
  });
});

describe("settings shortcuts", () => {
  it("only offers Reset all when a key was changed", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("nav-settings"));
    fireEvent.click(await screen.findByTestId("set-shortcuts"));
    const reset = screen.getByTestId<HTMLButtonElement>("bind-reset-all");
    expect(reset.disabled).toBe(true);
    fireEvent.click(screen.getByTestId("bind-split"));
    fireEvent.keyDown(window, { key: "x" });
    await waitFor(() => expect(reset.disabled).toBe(false));
    fireEvent.click(reset);
    expect(reset.disabled).toBe(true);
    expect(screen.getByTestId("bind-split").textContent).toBe("S");
  });
});
