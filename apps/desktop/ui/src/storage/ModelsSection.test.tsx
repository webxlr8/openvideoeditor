import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";
import { sizeText } from "./ModelsSection";

async function openStorage() {
  render(<App />);
  fireEvent.click(screen.getByTestId("nav-settings"));
  fireEvent.click(await screen.findByTestId("set-storage"));
}

describe("settings: storage models", () => {
  it("formats sizes", () => {
    expect(sizeText(147_951_465)).toBe("148.0 MB");
    expect(sizeText(120e9)).toBe("120.0 GB");
    expect(sizeText(null)).toBe("…");
  });

  it("lists the models and removes one; captions then report it missing", async () => {
    window.__oveMock!.captions = { installed: ["base"], stepMs: 1 };
    await openStorage();
    const state = await screen.findByTestId("model-state-whisper-base");
    expect(state.textContent).toBe("installed");
    expect(screen.getByTestId("model-whisper-base").textContent).toContain(
      "148.0 MB",
    );
    expect(screen.getByTestId("model-whisper-base").textContent).toContain(
      "whisper-base",
    );
    expect(screen.getByTestId("model-state-whisper-small").textContent).toBe(
      "not installed",
    );
    expect(screen.queryByTestId("model-remove-whisper-small")).toBeNull();

    fireEvent.click(screen.getByTestId("model-remove-whisper-base"));
    await waitFor(() =>
      expect(screen.getByTestId("model-state-whisper-base").textContent).toBe(
        "not installed",
      ),
    );
    expect(screen.getByTestId("models-message").textContent).toMatch(
      /Removed whisper-base: 148\.0 MB freed, .* free\./,
    );

    // The captions feature now reports the speech model as missing (it would download it again).
    const models = await commands.captionModels();
    expect(
      models.status === "ok" &&
        models.data.find((m) => m.id === "base")?.installed,
    ).toBe(false);
  });

  it("waits for the running captions job before removing the model it uses", async () => {
    window.__oveMock!.captions = { installed: ["tiny"], stepMs: 150 };
    // A captions job with the tiny model is running (four transcription steps).
    const started = await commands.downloadCaptionModel("tiny");
    expect(started.status).toBe("ok");
    await openStorage();
    await waitFor(() =>
      expect(screen.getByTestId("model-state-whisper-tiny").textContent).toBe(
        "in use",
      ),
    );

    fireEvent.click(screen.getByTestId("model-remove-whisper-tiny"));
    await waitFor(() =>
      expect(screen.getByTestId("models-message").textContent).toContain(
        "is in use",
      ),
    );
    expect(screen.getByTestId("model-remove-whisper-tiny").textContent).toBe(
      "Waiting…",
    );
    expect(window.__oveMock!.captions.installed).toContain("tiny");

    // When the job ends, the model is removed and the list says so.
    await waitFor(
      () =>
        expect(screen.getByTestId("model-state-whisper-tiny").textContent).toBe(
          "not installed",
        ),
      {
        timeout: 3000,
      },
    );
    expect(screen.getByTestId("models-message").textContent).toContain(
      "Removed whisper-tiny",
    );
    expect(window.__oveMock!.captions.installed).not.toContain("tiny");
  });
});
