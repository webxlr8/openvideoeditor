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
});

describe("ChatPanel", () => {
  it("streams a turn: text, tool chips, a timeline edit and a cost footer", async () => {
    const composer = await openBlankProject();
    type(composer, "Add an opening title");
    expect(screen.getByTestId("msg-user").textContent).toBe("Add an opening title");
    expect(await screen.findByTestId("stop")).toBeTruthy();
    await waitFor(() => expect(screen.getAllByTestId("tool-chip").length).toBe(3), { timeout: 3000 });
    const footer = await screen.findByTestId("chat-footer", {}, { timeout: 3000 });
    expect(footer.textContent).toMatch(/≈\$0\.01/);
    const reply = screen.getAllByTestId("msg-assistant").map((m) => m.textContent).join(" ");
    expect(reply).toContain("Done: I added a 3-second Welcome title");
    expect(screen.getByTestId("lane-Text").textContent).toContain("Welcome");
    expect(screen.queryByTestId("stop")).toBeNull();
    // The conversation is in the history menu.
    fireEvent.click(screen.getByTestId("chat-history-button"));
    expect(screen.getByTestId("chat-history-menu").textContent).toContain("Add an opening title");
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

  it("asks for a key when none is stored, and validates it", async () => {
    window.__oveMock!.agent.hasKey = false;
    await openBlankProject();
    const input = await screen.findByTestId("key-input");
    expect(screen.getByTestId("key-card").textContent).toContain("your own Anthropic API key");
    expect((screen.getByTestId("send") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(input, { target: { value: "not-a-key" } });
    fireEvent.click(screen.getByTestId("key-save"));
    expect(await screen.findByText(/start with sk-ant-/)).toBeTruthy();
    fireEvent.change(input, { target: { value: "sk-ant-test-0123456789" } });
    fireEvent.click(screen.getByTestId("key-save"));
    await waitFor(() => expect(screen.queryByTestId("key-card")).toBeNull());
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
