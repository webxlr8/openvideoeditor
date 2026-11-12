import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "./App";
import { MarkdownLite } from "./ChatPanel";

async function openBlankProject() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-blank"));
  await screen.findByTestId("editor");
  return screen.findByTestId("composer");
}

function type(el: HTMLElement, text: string) {
  fireEvent.change(el, { target: { value: text } });
  fireEvent.keyDown(el, { key: "Enter" });
}

beforeEach(() => {
  const agent = window.__oveMock!.agent;
  agent.hasKey = true;
  agent.stepMs = 5;
  agent.ask = false;
  agent.keyWorks = true;
  agent.turnCost = undefined;
});

describe("ChatPanel", () => {
  it("streams a turn: text, tool chips, a timeline edit and a cost footer", async () => {
    const composer = await openBlankProject();
    type(composer, "Add an opening title");
    expect(screen.getByTestId("msg-user").textContent).toBe("Add an opening title");
    expect(await screen.findByTestId("stop")).toBeTruthy();
    await waitFor(() => expect(screen.getAllByTestId("tool-chip").length).toBe(4), { timeout: 3000 });
    const footer = await screen.findByTestId("chat-footer", {}, { timeout: 3000 });
    expect(footer.textContent).toMatch(/≈\$0\.01/);
    const reply = screen.getAllByTestId("msg-assistant").map((m) => m.textContent).join(" ");
    expect(reply).toContain("Done: I added a 3-second Welcome title");
    expect(screen.getByTestId("lane-Text").textContent).toContain("Welcome");
    // seek_viewer moved the viewer to 1.5 s.
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:01:15"));
    expect(screen.queryByTestId("stop")).toBeNull();
    // The conversation is in the history menu.
    fireEvent.click(screen.getByTestId("chat-history-button"));
    expect(screen.getByTestId("chat-history-menu").textContent).toContain("Add an opening title");
  });

  it("reviews a finished turn: what changed, highlighted on the timeline, undone in one step", async () => {
    const composer = await openBlankProject();
    type(composer, "Add an opening title");
    const review = await screen.findByTestId("turn-review", {}, { timeout: 3000 });
    expect(review.textContent).toContain("Changed 1 item");
    // Cost and tokens of the turn, and running totals in the header.
    expect((await screen.findByTestId("turn-usage")).textContent).toMatch(/5\.4k in · 320 out · ≈\$0\.01/);
    await waitFor(() => expect(screen.getByTestId("chat-usage").textContent).toMatch(/This chat: 5\.4k in · 320 out · ≈\$0\.01/));
    const reviewed = () => screen.queryAllByTestId("clip").filter((c) => c.classList.contains("reviewed"));
    expect(reviewed().length).toBe(0);
    fireEvent.click(screen.getByTestId("turn-show"));
    await waitFor(() => expect(reviewed().length).toBe(1));
    expect(reviewed()[0].textContent).toContain("Welcome");
    fireEvent.click(screen.getByTestId("turn-show"));
    await waitFor(() => expect(reviewed().length).toBe(0));
    fireEvent.click(screen.getByTestId("turn-undo"));
    await waitFor(() => expect(screen.getByTestId("lane-Text").textContent).not.toContain("Welcome"));
    await waitFor(() => expect(screen.getByTestId("turn-review").textContent).toContain("Undone"));
    expect(screen.queryByTestId("turn-undo")).toBeNull();
    // Undoing the turn is itself one undo step.
    expect(screen.getByTestId("undo").hasAttribute("disabled")).toBe(false);
  });

  it("checks a new key with a real request and says why it failed", async () => {
    window.__oveMock!.agent.hasKey = false;
    window.__oveMock!.agent.keyWorks = false;
    render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    fireEvent.click(screen.getByTestId("toggle-chat"));
    const card = await screen.findByTestId("key-card");
    expect(screen.getByTestId("assistant-data").textContent).toContain("Never sent: your media files or video frames");
    fireEvent.change(screen.getByTestId("key-input"), { target: { value: "sk-ant-test-0123456789" } });
    fireEvent.click(screen.getByTestId("key-save"));
    expect((await screen.findByTestId("key-error")).textContent).toContain("rejected");
    expect(card.isConnected).toBe(true);
    // A working key gets a plain confirmation before the first message.
    window.__oveMock!.agent.keyWorks = true;
    fireEvent.change(screen.getByTestId("key-input"), { target: { value: "sk-ant-test-0123456789" } });
    fireEvent.click(screen.getByTestId("key-save"));
    expect((await screen.findByTestId("key-check")).textContent).toContain("Key works");
  });

  it("Shift+Enter keeps typing instead of sending", async () => {
    const composer = await openBlankProject();
    fireEvent.change(composer, { target: { value: "line one" } });
    fireEvent.keyDown(composer, { key: "Enter", shiftKey: true });
    expect(screen.queryByTestId("msg-user")).toBeNull();
  });

  it("stops a running turn", async () => {
    window.__oveMock!.agent.stepMs = 40;
    const composer = await openBlankProject();
    type(composer, "Make a teaser");
    fireEvent.click(await screen.findByTestId("stop"));
    const footer = await screen.findByTestId("chat-footer", {}, { timeout: 3000 });
    expect(footer.textContent).toBe("Stopped.");
    expect(screen.getByTestId("lane-Text").textContent).not.toContain("Welcome");
    expect(await screen.findByTestId("send")).toBeTruthy();
  });

  it("stays closed without a key until opened, then asks for a key and validates it", async () => {
    window.__oveMock!.agent.hasKey = false;
    render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    // Not pushed on people who haven't set the assistant up; the toggle stays available.
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(screen.queryByTestId("chat")).toBeNull();
    fireEvent.click(screen.getByTestId("toggle-chat"));
    const input = await screen.findByTestId("key-input");
    expect(screen.getByTestId("key-card").textContent).toContain("your own Anthropic API key");
    expect(screen.getByTestId<HTMLButtonElement>("send").disabled).toBe(true);
    fireEvent.change(input, { target: { value: "not-a-key" } });
    fireEvent.click(screen.getByTestId("key-save"));
    expect(await screen.findByText(/start with sk-ant-/)).toBeTruthy();
    fireEvent.change(input, { target: { value: "sk-ant-test-0123456789" } });
    fireEvent.click(screen.getByTestId("key-save"));
    await waitFor(() => expect(screen.queryByTestId("key-card")).toBeNull());
  });

  it("remembers whether the person opened or closed the assistant", async () => {
    window.__oveMock!.agent.hasKey = false;
    const { unmount } = render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    fireEvent.click(screen.getByTestId("toggle-chat"));
    await screen.findByTestId("key-card");
    unmount();

    // Next launch: opened last time, so it opens again even without a key.
    const second = render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    expect(screen.getByTestId("chat")).toBeTruthy();
    fireEvent.click(screen.getByTestId("close-chat"));
    second.unmount();

    // Closed by the person: stays closed, even with a key.
    window.__oveMock!.agent.hasKey = true;
    render(<App />);
    fireEvent.click(screen.getByTestId("home-blank"));
    await screen.findByTestId("editor");
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(screen.queryByTestId("chat")).toBeNull();
  });

  it("shows a question card and continues with the answer", async () => {
    window.__oveMock!.agent.ask = true;
    const composer = await openBlankProject();
    type(composer, "Add a title");
    const card = await screen.findByTestId("question-card", {}, { timeout: 3000 });
    expect(card.textContent).toContain("Which style should the title use?");
    await act(async () => fireEvent.click(screen.getByText("Minimal")));
    await screen.findByTestId("chat-footer", {}, { timeout: 3000 });
    expect(screen.getByTestId("question-card").textContent).toContain("→ Minimal");
    expect(screen.getByTestId("lane-Text").textContent).toContain("Welcome");
  });

  it("sends the Home prompt as the first message of a new project", async () => {
    render(<App />);
    const prompt = screen.getByTestId("home-prompt");
    fireEvent.change(prompt, { target: { value: "A travel diary from Lisbon" } });
    fireEvent.click(screen.getByTestId("home-send"));
    await screen.findByTestId("editor");
    await screen.findByTestId("chat-footer", {}, { timeout: 3000 });
    expect(screen.getByTestId("msg-user").textContent).toBe("A travel diary from Lisbon");
    expect(screen.getByTestId("project-name").textContent).toContain("A travel diary from Lisbon");
  });
});

describe("MarkdownLite", () => {
  it("renders paragraphs, lists, bold and code", () => {
    const { container } = render(<MarkdownLite text={"Plan:\n- cut **intro**\n- trim `i3`\n\n1. one\n2. two"} />);
    expect(container.querySelectorAll("ul li").length).toBe(2);
    expect(container.querySelectorAll("ol li").length).toBe(2);
    expect(container.querySelector("strong")?.textContent).toBe("intro");
    expect(container.querySelector("code")?.textContent).toBe("i3");
  });
});
