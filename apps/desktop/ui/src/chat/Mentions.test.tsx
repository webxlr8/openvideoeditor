import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";
import { type MentionDraft, filterMentions, mentionQuery, slashQuery } from "./mentions";

beforeEach(() => {
  const agent = window.__oveMock!.agent;
  agent.hasKey = true;
  agent.stepMs = 5;
  agent.ask = false;
  agent.sent = undefined;
});

/** The editor with the sample clip and the assistant open. */
async function editorWithChat() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  if (!screen.queryByTestId("composer")) fireEvent.click(screen.getByTestId("toggle-chat"));
  return screen.findByTestId<HTMLTextAreaElement>("composer");
}

function typeInto(el: HTMLTextAreaElement, value: string) {
  fireEvent.change(el, { target: { value, selectionStart: value.length } });
}

const rows = () => screen.queryAllByTestId("mention-menu-row").map((r) => r.textContent ?? "");

describe("mentions", () => {
  it("filters by tab and text, and finds the @ being typed", () => {
    const all: MentionDraft[] = [
      { kind: "media", id: "m1", label: "Beach.mov", media: "video" },
      { kind: "media", id: "m2", label: "Poster.png", media: "image" },
      { kind: "media", id: "m3", label: "Theme.wav", media: "audio" },
      { kind: "item", id: "i1", label: "Opening title", media: "text" },
      { kind: "range", start: 1, end: 2, label: "Marked range 1s–2s" },
    ];
    expect(filterMentions(all, "", "all").length).toBe(5);
    expect(filterMentions(all, "", "image").map((m) => m.id)).toEqual(["m2"]);
    expect(filterMentions(all, "", "audio").map((m) => m.id)).toEqual(["m3"]);
    expect(filterMentions(all, "TITLE", "all").map((m) => m.id)).toEqual(["i1"]);
    expect(filterMentions(all, "title", "video")).toEqual([]);
    expect(mentionQuery("move @Ope")).toEqual({ query: "Ope", at: 5 });
    expect(mentionQuery("@")).toEqual({ query: "", at: 0 });
    expect(mentionQuery("mail me@home")).toBeNull();
    expect(mentionQuery("done @x ")).toBeNull();
    expect(slashQuery("/col")).toBe("col");
    expect(slashQuery("x /col")).toBeNull();
    expect(slashQuery("/col match")).toBeNull();
  });

  it("opens on @, moves with the arrow keys, picks with Enter and sends the mention", async () => {
    const composer = await editorWithChat();
    await act(async () => {
      await commands.importMedia(["/mock/Theme.wav"]);
    });
    typeInto(composer, "Make @");
    expect(await screen.findByTestId("mention-menu")).toBeTruthy();
    expect(rows().some((r) => r.includes("Sample clip.mov"))).toBe(true);
    expect(rows().some((r) => r.includes("Theme.wav"))).toBe(true);
    // The Audio tab lists only sound.
    fireEvent.click(screen.getByTestId("mention-menu-tab-audio"));
    expect(rows().every((r) => r.includes("Theme"))).toBe(true);
    fireEvent.click(screen.getByTestId("mention-menu-tab-all"));
    typeInto(composer, "Make @Sample");
    expect(rows().length).toBeGreaterThanOrEqual(2); // the media file and its clip
    const options = () => within(screen.getByTestId("mention-menu")).getAllByRole("option");
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(composer, { key: "ArrowDown" });
    expect(options()[1].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(composer, { key: "ArrowUp" });
    fireEvent.keyDown(composer, { key: "Enter" });
    expect(screen.queryByTestId("mention-menu")).toBeNull();
    expect(screen.getByTestId("mention-chip").textContent).toContain("Sample clip.mov");
    expect(composer.value).toBe("Make ");
    // Escape closes the popover without picking.
    typeInto(composer, "Make @Sam");
    fireEvent.keyDown(composer, { key: "Escape" });
    expect(screen.queryByTestId("mention-menu")).toBeNull();
    typeInto(composer, "Make it brighter");
    fireEvent.keyDown(composer, { key: "Enter" });
    const sent = await screen.findByTestId("msg-mentions");
    expect(sent.textContent).toContain("Sample clip.mov");
    const sentMentions = window.__oveMock!.agent.sent ?? [];
    expect(sentMentions.map((m) => [m.kind, m.start, m.end])).toEqual([["media", null, null]]);
    expect(sentMentions[0].id).toBeTruthy();
    expect(screen.queryByTestId("composer-mentions")).toBeNull();
    // The turn finishes (the next test starts with the assistant idle).
    await waitFor(() => expect(screen.queryByTestId("stop")).toBeNull(), { timeout: 3000 });
  });

  it("attaches at most five images and says which were left out", async () => {
    const composer = await editorWithChat();
    const names = Array.from({ length: 7 }, (_, i) => `poster${i + 1}.png`);
    await act(async () => {
      await commands.importMedia(names.map((n) => `/mock/${n}`));
    });
    for (const n of names) {
      typeInto(composer, `@${n}`);
      fireEvent.click(screen.getByTestId("mention-menu-tab-image"));
      expect(rows()[0]).toContain(n);
      fireEvent.keyDown(composer, { key: "Tab" });
    }
    expect(screen.getAllByTestId("mention-chip").length).toBe(7);
    typeInto(composer, "Use these as a slideshow");
    fireEvent.keyDown(composer, { key: "Enter" });
    const notice = await screen.findByTestId("chat-notice", {}, { timeout: 3000 });
    expect(notice.textContent).toContain("Only 5 images can be attached");
    expect(notice.textContent).toContain("poster6.png");
    expect(notice.textContent).toContain("poster7.png");
    expect(notice.textContent).not.toContain("poster5.png");
  });
});
