import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";

beforeEach(() => {
  const agent = window.__oveMock!.agent;
  agent.hasKey = true;
  agent.stepMs = 5;
  agent.ask = false;
  agent.sent = undefined;
});

async function editor() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
}

/** Opens a panel, picks one of its prepared requests, and returns the chat input. */
async function handOff(panel: string, entry: string) {
  fireEvent.click(screen.getByTestId(`rail-${panel}`));
  fireEvent.click(await screen.findByTestId(`ask-${panel}-button`));
  fireEvent.click(within(screen.getByTestId(`ask-${panel}-menu`)).getByText(entry));
  const composer = await screen.findByTestId<HTMLTextAreaElement>("composer");
  await waitFor(() => expect(composer.value).not.toBe(""));
  return composer;
}

const chips = () => screen.queryAllByTestId("mention-chip").map((c) => c.textContent ?? "");

/** Nothing was sent: no user message, and the mock assistant received nothing. */
function nothingSent() {
  expect(screen.queryByTestId("msg-user")).toBeNull();
  expect(window.__oveMock!.agent.sent).toBeUndefined();
}

describe("panel hand-off", () => {
  it("Captions: Remove filler words opens the chat with the request and the caption track", async () => {
    await editor();
    await act(async () => {
      await commands.addItems(
        JSON.stringify([{ trackId: "t_captions", type: "caption", startTime: 0, endTime: 2, name: "Caption", captionText: "um so hi" }]),
        "add caption",
      );
    });
    const composer = await handOff("captions", "Remove filler words");
    expect(composer.value).toContain("Remove the filler words");
    expect(chips()).toEqual([expect.stringContaining("Captions track")]);
    nothingSent();
    // The person can edit it, then sends it: the track mention goes with it.
    fireEvent.change(composer, { target: { value: `${composer.value} Keep "like" when it compares.` } });
    fireEvent.keyDown(composer, { key: "Enter" });
    await screen.findByTestId("msg-user");
    expect(window.__oveMock!.agent.sent?.map((m) => m.kind)).toEqual(["track"]);
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 3000 });
  });

  it("Audio: Score this range mentions the marked range and the selected clip", async () => {
    await editor();
    fireEvent.click(screen.getAllByTestId("clip")[0]);
    // Mark the clip as the range (X).
    fireEvent.keyDown(window, { key: "x" });
    const composer = await handOff("audio", "Score this range");
    expect(composer.value).toContain("Add music under this range");
    await waitFor(() => expect(chips().length).toBe(2));
    expect(chips()[0]).toMatch(/^Marked range 0s–/);
    nothingSent();
  });

  it("Media: Organise the media puts the request in the input", async () => {
    await editor();
    const composer = await handOff("media", "Organise the media");
    expect(composer.value).toContain("suggest clear names");
    expect(chips()).toEqual([]);
    nothingSent();
  });

  it("Effects: Match this shot mentions the selected clips", async () => {
    await editor();
    fireEvent.click(screen.getAllByTestId("clip")[0]);
    const composer = await handOff("effects", "Match this shot to a reference");
    expect(composer.value).toContain("Match the colour");
    expect(chips()).toEqual([expect.stringContaining("Sample clip")]);
    nothingSent();
  });

  it("Transcript: Tighten the edit puts the request in the input", async () => {
    await editor();
    const composer = await handOff("transcript", "Tighten the edit");
    expect(composer.value).toContain("Tighten this part");
    nothingSent();
  });
});

describe("the / menu", () => {
  async function composer() {
    await editor();
    if (!screen.queryByTestId("composer")) fireEvent.click(screen.getByTestId("toggle-chat"));
    return screen.findByTestId<HTMLTextAreaElement>("composer");
  }
  const rows = () => within(screen.getByTestId("slash-menu")).queryAllByTestId("slash-menu-row").map((r) => r.textContent ?? "");

  it("lists skills and templates, filters as you type, and a skill becomes a chip", async () => {
    const c = await composer();
    fireEvent.change(c, { target: { value: "/" } });
    await waitFor(() => expect(rows().some((r) => r.includes("Skill"))).toBe(true));
    expect(rows().some((r) => r.includes("Template"))).toBe(true);
    const skill = rows().find((r) => r.includes("Skill"))!.replace("Skill", "");
    fireEvent.change(c, { target: { value: `/${skill.slice(0, 4)}` } });
    expect(rows()[0]).toContain(skill);
    fireEvent.keyDown(c, { key: "Enter" });
    expect(screen.queryByTestId("slash-menu")).toBeNull();
    expect(chips()).toEqual([skill]);
    expect(c.value).toBe("");
    fireEvent.change(c, { target: { value: "match these two shots" } });
    fireEvent.keyDown(c, { key: "Enter" });
    await screen.findByTestId("msg-user");
    expect(window.__oveMock!.agent.sent?.[0].kind).toBe("skill");
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 3000 });
  });

  it("a template opens its intake form and puts its request in the input", async () => {
    const c = await composer();
    fireEvent.change(c, { target: { value: "/" } });
    const template = rows().find((r) => r.includes("Template"))!.replace("Template", "");
    fireEvent.change(c, { target: { value: `/${template.split(" ")[0]}` } });
    const row = within(screen.getByTestId("slash-menu"))
      .getAllByTestId("slash-menu-row")
      .find((r) => r.textContent?.includes(template))!;
    fireEvent.click(row);
    const intake = await screen.findByTestId("template-intake");
    fireEvent.click(within(intake).getByTestId("intake-done"));
    // Required details are asked for first.
    expect(screen.getByTestId("intake-missing")).toBeTruthy();
    fireEvent.change(within(intake).getByTestId("intake-subject"), { target: { value: "Our new bakery" } });
    for (const el of intake.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input, textarea"))
      if (!el.value) fireEvent.change(el, { target: { value: "Something" } });
    fireEvent.click(within(intake).getByTestId("intake-done"));
    expect(screen.queryByTestId("template-intake")).toBeNull();
    expect(c.value).toContain("Our new bakery");
    nothingSent();
  });

  it("with no match it says so, and Enter sends the text as typed", async () => {
    const c = await composer();
    fireEvent.change(c, { target: { value: "/zzzz-nothing" } });
    expect(await screen.findByTestId("slash-menu-empty")).toBeTruthy();
    expect(screen.getByTestId("slash-menu-empty").textContent).toBe("No matches");
    fireEvent.keyDown(c, { key: "Enter" });
    expect((await screen.findByTestId("msg-user")).textContent).toBe("/zzzz-nothing");
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 3000 });
  });
});
