import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { keyToAction, resetBindings } from "./keys";

async function openGenerate() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  fireEvent.click(screen.getByTestId("rail-generate"));
  return screen.findByTestId("generate-panel");
}

describe("generate panel", () => {
  it("asks for a key, shows the estimate, needs confirmation, and fills the placeholder", async () => {
    await openGenerate();
    // No fal.ai key yet: the key entry is shown and Generate is disabled.
    const keyBox = await screen.findByTestId("key-fal");
    fireEvent.change(screen.getByTestId("gen-prompt"), {
      target: { value: "A lighthouse at dawn" },
    });
    await screen.findByTestId("gen-estimate");
    expect(screen.getByTestId("gen-estimate").textContent).toContain("$0.025");
    expect(
      (screen.getByTestId("gen-start") as HTMLButtonElement).disabled,
    ).toBe(true);
    fireEvent.change(keyBox.querySelector("input")!, {
      target: { value: "fal-key-0123456789abcdef" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(screen.queryByTestId("key-fal")).toBeNull());

    // First click asks to confirm the spend; only the second starts the job.
    fireEvent.click(screen.getByTestId("gen-start"));
    expect(screen.getByTestId("gen-start").textContent).toContain(
      "Confirm: spend",
    );
    expect(
      window.__oveMock!.calls.some((c) => c.startsWith("startGeneration")),
    ).toBe(false);
    fireEvent.click(screen.getByTestId("gen-start"));
    await waitFor(() =>
      expect(window.__oveMock!.calls).toContain("startGeneration:image"),
    );
    await screen.findByTestId("gen-job");
    await waitFor(
      () => expect(screen.getByTestId("gen-job").textContent).toContain("done"),
      { timeout: 3000 },
    );
  });

  it("switches kinds with their own options", async () => {
    await openGenerate();
    fireEvent.click(screen.getByTestId("gen-kind-video"));
    expect(screen.getByText("5 s")).toBeTruthy();
    fireEvent.click(screen.getByTestId("gen-kind-voice"));
    expect(screen.getByTestId("key-elevenlabs")).toBeTruthy();
  });
});

describe("settings", () => {
  it("opens from home, stores provider keys and rebinds shortcuts", async () => {
    resetBindings();
    render(<App />);
    fireEvent.click(screen.getByTestId("nav-settings"));
    await screen.findByTestId("settings");
    fireEvent.click(screen.getByTestId("set-generation"));
    const el = await screen.findByTestId("key-elevenlabs");
    fireEvent.change(el.querySelector("input")!, {
      target: { value: "el-key-0123456789abcdef" },
    });
    fireEvent.click(el.querySelector("button")!);
    await waitFor(() =>
      expect(screen.getByTestId("key-elevenlabs").textContent).toContain(
        "key saved",
      ),
    );

    fireEvent.click(screen.getByTestId("set-shortcuts"));
    fireEvent.click(await screen.findByTestId("bind-split"));
    expect(screen.getByTestId("bind-split").textContent).toBe("Press a key…");
    fireEvent.keyDown(window, { key: "x" });
    await waitFor(() =>
      expect(screen.getByTestId("bind-split").textContent).toBe("X"),
    );
    const k = {
      shiftKey: false,
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      target: document.body,
    };
    expect(keyToAction({ ...k, key: "x" })).toEqual({ type: "split" });
    resetBindings();
  });
});

describe("templates", () => {
  it("starts a project from a template with the frame shape and a brief", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("nav-templates"));
    await screen.findByTestId("templates-page");
    fireEvent.click(screen.getByTestId("template-podcast-clip"));
    fireEvent.change(await screen.findByTestId("template-details"), {
      target: { value: "Harbour Talks, episode 12" },
    });
    fireEvent.click(screen.getByTestId("template-start"));
    await screen.findByTestId("editor");
    await waitFor(() =>
      expect(screen.getByTestId("project-bar").textContent).toContain("9:16"),
    );
    expect(screen.getByTestId("project-bar").textContent).toContain(
      "Podcast clip – Harbour Talks",
    );
  });
});
