import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";

/** The editor with its video clip on Main and a music clip on the Audio track. */
async function editorWithMusic() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
  const view = await commands.projectView();
  if (view.status !== "ok" || !view.data) throw new Error("no project");
  const p = view.data;
  const clip = p.items[0];
  const audio = p.tracks.find((t) => t.kind === "audio")!;
  await act(async () => {
    await commands.addItems(JSON.stringify([{ trackId: audio.id, type: "audio", startTime: 0, endTime: clip.end, mediaItemId: clip.mediaId, name: "Score" }]), null);
  });
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(2));
  return { main: clip.trackId, audio: audio.id, clipId: clip.id };
}

const musicPoints = () => {
  const music = screen.getAllByTestId("clip").find((c) => c.textContent?.includes("Score"))!;
  return music.querySelectorAll('[data-testid="gain-point"]').length;
};

describe("duck popover", () => {
  it("previews, ducks the music as one revision with volume keyframes, and undo removes them", async () => {
    const { main, audio } = await editorWithMusic();
    fireEvent.click(screen.getByTestId("rail-audio"));
    fireEvent.click(await screen.findByTestId("audio-duck-open"));
    await screen.findByTestId("duck-popover");
    expect(screen.getByTestId<HTMLSelectElement>("duck-music").value).toBe(audio);
    fireEvent.change(screen.getByTestId("duck-under"), { target: { value: main } });
    fireEvent.change(screen.getByTestId("duck-depth"), { target: { value: "-15" } });
    expect(screen.getByTestId("duck-depth-value").textContent).toBe("-15 dB");
    fireEvent.click(screen.getByTestId("duck-preview-run"));
    const preview = await screen.findByTestId("duck-preview");
    expect(preview.textContent).toContain("Found 1 line of speech");
    expect(musicPoints()).toBe(0);
    fireEvent.click(screen.getByTestId("duck-apply"));
    await waitFor(() => expect(screen.queryByTestId("duck-popover")).toBeNull());
    await waitFor(() => expect(musicPoints()).toBe(4));
    fireEvent.click(screen.getByTestId("undo"));
    await waitFor(() => expect(musicPoints()).toBe(0));
  });

  it("says when no speech was found and changes nothing", async () => {
    const { main, clipId } = await editorWithMusic();
    await act(async () => {
      await commands.updateItem(clipId, JSON.stringify({ muted: true }), [], null);
    });
    fireEvent.click(screen.getByTestId("rail-audio"));
    fireEvent.click(await screen.findByTestId("audio-duck-open"));
    fireEvent.change(await screen.findByTestId("duck-under"), { target: { value: main } });
    fireEvent.click(screen.getByTestId("duck-apply"));
    const err = await screen.findByTestId("duck-error");
    expect(err.textContent).toContain("No speech was found");
    expect(screen.getByTestId("duck-popover")).toBeTruthy();
    expect(musicPoints()).toBe(0);
  });
});
