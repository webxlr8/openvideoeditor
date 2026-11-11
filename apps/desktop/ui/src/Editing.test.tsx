import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";

async function editorWithClip() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  await screen.findByTestId("inspector");
}

const clips = () => screen.queryAllByTestId("clip");

describe("editing", () => {
  it("inspector commits a slider once on release and a field on Enter", async () => {
    await editorWithClip();
    const opacity = (await screen.findByTestId("insp-opacity")) as HTMLInputElement;
    fireEvent.change(opacity, { target: { value: "0.4" } });
    fireEvent.change(opacity, { target: { value: "0.3" } });
    fireEvent.pointerUp(opacity);
    await waitFor(() => expect(screen.getByTestId("undo").hasAttribute("disabled")).toBe(false));
    const x = screen.getByTestId("insp-x") as HTMLInputElement;
    fireEvent.change(x, { target: { value: "120" } });
    fireEvent.keyDown(x, { key: "Enter" });
    fireEvent.blur(x);
    await waitFor(() => expect((screen.getByTestId("insp-x") as HTMLInputElement).value).toBe("120"));
    expect(Number((screen.getByTestId("insp-opacity") as HTMLInputElement).value)).toBeCloseTo(0.3);
  });

  it("adds a title from the Text panel and edits it in the inspector", async () => {
    await editorWithClip();
    fireEvent.click(screen.getByTestId("rail-text"));
    fireEvent.click(await screen.findByTestId("text-preset-headline"));
    await waitFor(() => expect(clips().length).toBe(2));
    expect(screen.getByTestId("track-Text").textContent).toContain("Big Story");
    // Select the new title: its Text tab opens.
    const title = clips().find((c) => c.textContent?.includes("Big Story"))!;
    fireEvent.pointerDown(title, { button: 0, clientX: 300 });
    fireEvent.pointerUp(window);
    const text = (await screen.findByTestId("insp-text")) as HTMLTextAreaElement;
    fireEvent.change(text, { target: { value: "Hello world" } });
    fireEvent.blur(text);
    await waitFor(() => expect(screen.getByTestId("track-Text").textContent).toContain("Hello world"));
  });

  it("trims the start edge, moving the in-point", async () => {
    await editorWithClip();
    const handle = screen.getByTestId("trim-start");
    fireEvent.pointerDown(handle, { button: 0, clientX: 200 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 260 }));
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 260 }));
    });
    await waitFor(() => expect(screen.getByTestId("undo").hasAttribute("disabled")).toBe(false));
    expect(clips()[0].getAttribute("title")).not.toContain("00:00.00–");
  });

  it("right-click menu duplicates and deletes", async () => {
    await editorWithClip();
    fireEvent.contextMenu(clips()[0], { clientX: 300, clientY: 600 });
    fireEvent.click(await screen.findByTestId("menu-duplicate"));
    await waitFor(() => expect(clips().length).toBe(2));
    fireEvent.contextMenu(clips()[1], { clientX: 300, clientY: 600 });
    fireEvent.click(await screen.findByTestId("menu-delete"));
    await waitFor(() => expect(clips().length).toBe(1));
  });

  it("keyboard: D duplicates, ⌘C/⌘V pastes, S splits, ? shows shortcuts", async () => {
    await editorWithClip();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(clips().length).toBe(2));
    fireEvent.keyDown(window, { key: "c", ctrlKey: true });
    await act(async () => new Promise((r) => setTimeout(r, 20)));
    fireEvent.keyDown(window, { key: "v", ctrlKey: true });
    await waitFor(() => expect(clips().length).toBe(3));
    // Move the playhead into the selected clip, then split.
    for (let i = 0; i < 30; i++) fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => expect(screen.getByTestId("timecode").textContent).toBe("00:00:01:00"));
    fireEvent.keyDown(window, { key: "s" });
    await waitFor(() => expect(clips().length).toBe(4));
    fireEvent.keyDown(window, { key: "?" });
    expect(await screen.findByTestId("shortcuts-overlay")).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("shortcuts-overlay")).toBeNull());
  });

  it("panels: transitions, looks, layout and captions", async () => {
    await editorWithClip();
    fireEvent.click(screen.getByTestId("rail-transitions"));
    const fadeCard = await screen.findByTestId("transition-fade");
    fireEvent.click(fadeCard.querySelector("button")!);
    await waitFor(() => expect(screen.getByTestId("undo").hasAttribute("disabled")).toBe(false));
    fireEvent.click(screen.getByTestId("rail-effects"));
    fireEvent.click(await screen.findByTestId("look-warm-layer"));
    await waitFor(() => expect(clips().length).toBe(2));
    fireEvent.click(screen.getByTestId("rail-layout"));
    fireEvent.click(await screen.findByTestId("aspect-9x16"));
    await waitFor(() => expect(screen.getByTestId("aspect-button").textContent).toContain("9:16"));
    expect(screen.getByTestId("aspect-9x16").classList.contains("on")).toBe(true);
    fireEvent.click(screen.getByTestId("rail-captions"));
    fireEvent.click(await screen.findByTestId("caption-add"));
    await waitFor(() => expect(screen.getByTestId("track-Captions").textContent).toContain("Your words here"));
  });

  it("shift-click adds to the selection and Delete removes all selected", async () => {
    await editorWithClip();
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(clips().length).toBe(2));
    const other = clips().find((c) => !c.classList.contains("selected"))!;
    fireEvent.pointerDown(other, { button: 0, shiftKey: true });
    await waitFor(() => expect(clips().filter((c) => c.classList.contains("selected")).length).toBe(2));
    fireEvent.keyDown(window, { key: "Delete" });
    await waitFor(() => expect(clips().length).toBe(0));
  });
});
