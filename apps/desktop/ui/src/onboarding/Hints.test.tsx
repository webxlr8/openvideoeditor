import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { RAIL } from "../Editor";
import { STARTERS } from "./StarterPrompts";

const mock = () => window.__oveMock!;
const clips = () => screen.queryAllByTestId("clip");

async function blankProject() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-blank"));
  await screen.findByTestId("editor");
}

async function projectWithClip() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(clips().length).toBe(1));
  // The imported clip is selected; start from nothing selected, on the Media panel.
  fireEvent.keyDown(window, { key: "Escape" });
  await screen.findByTestId("side-panel");
}

/** The hint of a panel: one sentence and one action. */
async function hint(panel: string) {
  fireEvent.click(screen.getByTestId(`rail-${panel}`));
  const el = await screen.findByTestId(`hint-${panel}`);
  const sentences = (el.querySelector("p")?.textContent ?? "").trim();
  expect(sentences, panel).toMatch(/\.$/);
  expect(within(el).getAllByRole("button").length, panel).toBe(1);
  return { el, action: within(el).getByRole("button") };
}

describe("hints in empty panels", () => {
  it("every panel of an empty project says what it is for and offers one action", async () => {
    await blankProject();
    const labels: Record<string, string> = {};
    for (const r of RAIL) {
      // Media is the panel a new project opens on.
      const { action } = r.id === "media" ? await hint("layout").then(() => hint("media")) : await hint(r.id);
      labels[r.id] = action.textContent ?? "";
    }
    // Motion offers its first template by name.
    expect(labels.motion).toMatch(/^Add “.+”$/);
    expect({ ...labels, motion: "" }).toEqual({
      media: "Import media",
      layout: "Import media",
      audio: "Import media",
      captions: "Import media",
      text: "Add a title",
      motion: "",
      transitions: "Import media",
      effects: "Import media",
      generate: "Try an example",
    });
  });

  it("the Media hint imports, and is gone once there is media", async () => {
    await blankProject();
    const { action } = await hint("layout").then(() => hint("media"));
    fireEvent.click(action);
    await waitFor(() => expect(clips().length).toBe(1));
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.getAllByTestId("media-card").length).toBe(1));
    expect(screen.queryByTestId("hint-media")).toBeNull();
  });

  it("the Captions hint says what captions do and offers to make them", async () => {
    await projectWithClip();
    const { el, action } = await hint("captions");
    expect(el.textContent).toContain("Captions put the spoken words on screen");
    expect(action.textContent).toBe("Make captions");
    mock().captions.stepMs = 5;
    fireEvent.click(action);
    await waitFor(() => expect(mock().calls.some((c) => c.startsWith("autoCaption:"))).toBe(true));
    // With captions in the project the panel is no longer empty.
    await waitFor(() => expect(clips().length).toBeGreaterThan(1), { timeout: 3000 });
    await waitFor(() => expect(screen.queryByTestId("hint-captions")).toBeNull());
  });

  it("the Text hint adds a title and goes away", async () => {
    await blankProject();
    const { action } = await hint("text");
    fireEvent.click(action);
    await waitFor(() => expect(clips().length).toBe(1));
    expect(screen.getByTestId("track-Text").textContent).toContain("Big Story");
    expect(screen.queryByTestId("hint-text")).toBeNull();
  });

  it("panels that work on a clip offer to select the first one, and stay open", async () => {
    await projectWithClip();
    for (const panel of ["transitions", "effects", "audio", "layout"]) {
      const { action } = await hint(panel);
      expect(action.textContent, panel).toBe("Select the first clip");
      fireEvent.click(action);
      await waitFor(() => expect(clips()[0].className, panel).toContain("selected"));
      // The panel is still there (the clip's properties did not take its place), and its hint is gone.
      expect(screen.getByTestId(`panel-${panel}`)).toBeTruthy();
      expect(screen.queryByTestId("inspector")).toBeNull();
      await waitFor(() => expect(screen.queryByTestId(`hint-${panel}`), panel).toBeNull());
      fireEvent.keyDown(window, { key: "Escape" });
      await waitFor(() => expect(clips()[0].className).not.toContain("selected"));
    }
  });

  it("the Generate hint fills in an example and spends nothing", async () => {
    await blankProject();
    const { action } = await hint("generate");
    fireEvent.click(action);
    const prompt = screen.getByTestId<HTMLTextAreaElement>("gen-prompt");
    expect(prompt.value.length).toBeGreaterThan(10);
    expect(screen.queryByTestId("hint-generate")).toBeNull();
    expect(screen.queryAllByTestId("gen-job").length).toBe(0);
  });
});

describe("starter prompts", () => {
  it("an empty chat offers three examples; tapping one fills the message box and sends nothing", async () => {
    await blankProject();
    const chat = await screen.findByTestId("chat");
    const starters = await within(chat).findByTestId("starters");
    expect(within(starters).getAllByTestId(/^starter-[123]$/).length).toBe(3);
    const composer = screen.getByTestId<HTMLTextAreaElement>("composer");
    expect(composer.value).toBe("");

    fireEvent.click(screen.getByTestId("starter-1"));
    expect(composer.value).toBe(STARTERS[0]);
    // Another tap replaces it; an example that ends open leaves room for the person's words.
    fireEvent.click(screen.getByTestId("starter-3"));
    expect(composer.value).toBe(STARTERS[2]);
    expect(composer.value.endsWith(" ")).toBe(true);
    // Nothing was sent: no message in the chat, the examples are still offered, Send is ready.
    expect(screen.queryByTestId("msg-user")).toBeNull();
    expect(screen.queryByTestId("chat-log")).toBeNull();
    expect(screen.getByTestId("starters")).toBeTruthy();
    expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false);
  });

  it("answers “What can you do?” in place", async () => {
    await blankProject();
    await screen.findByTestId("starters");
    expect(screen.queryByTestId("starter-answer")).toBeNull();
    fireEvent.click(screen.getByTestId("starter-what"));
    const answer = screen.getByTestId("starter-answer").textContent ?? "";
    expect(answer).toMatch(/titles, captions, transitions/);
    expect(answer).toMatch(/frames of its video/);
    expect(answer).toMatch(/one undo\s+step/);
    expect(screen.queryByTestId("msg-user")).toBeNull();
  });

  it("is gone once the conversation has a message", async () => {
    await blankProject();
    await screen.findByTestId("starters");
    mock().agent.stepMs = 1;
    fireEvent.click(screen.getByTestId("starter-2"));
    fireEvent.click(screen.getByTestId("send"));
    await screen.findByTestId("msg-user");
    expect(screen.queryByTestId("starters")).toBeNull();
    await screen.findByTestId("turn-usage", undefined, { timeout: 5000 });
  });
});
