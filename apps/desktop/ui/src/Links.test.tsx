import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";

async function editor() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
}

describe("linked items", () => {
  it("badges a linked pair and shows +3f when an edit outside the planners drifts it", async () => {
    await editor();
    await act(async () => {
      await commands.addItems(
        JSON.stringify([
          { id: "lv", trackId: "t_overlay", type: "video", startTime: 20, endTime: 24, trimStart: 1, link: "L1" },
          { id: "la", trackId: "t_audio", type: "audio", startTime: 20, endTime: 24, trimStart: 1, link: "L1" },
        ]),
        "add pair",
      );
    });
    await waitFor(() => expect(screen.getAllByTestId("link-badge").length).toBe(2));
    expect(screen.queryAllByTestId("drift-chip").length).toBe(0);
    // A file edit moves only the audio 3 frames (0.1 s at 30 fps) later.
    await act(async () => {
      await commands.updateItems(JSON.stringify([{ id: "la", set: { startTime: 20.1, endTime: 24.1 } }]), null);
    });
    await waitFor(() => expect(screen.getAllByTestId("drift-chip").map((c) => c.textContent)).toEqual(["+3f"]));
    const chip = screen.getByTestId("drift-chip");
    expect(chip.closest("[data-testid=clip]")?.getAttribute("data-id")).toBe("la");
  });
});
