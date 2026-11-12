import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { RELEASE_NOTES } from "./releaseNotes";
import { CRASH_OPT_IN_KEY, buildReport } from "./Support";

const CRASH = {
  path: "/logs/crash-reports/crash-1700000000000.txt",
  at: 1700000000000,
  summary: "index out of bounds",
  seen: true,
};

function stubClipboard() {
  const writeText = vi.fn(() => Promise.resolve());
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
  return writeText;
}

async function openFeedback() {
  render(<App />);
  fireEvent.click(screen.getByTestId("nav-feedback"));
  const dialog = await screen.findByTestId("feedback-dialog");
  // The environment block arrives from the core.
  await waitFor(() =>
    expect(screen.getByTestId("feedback-env").textContent).toContain(
      "startup: app",
    ),
  );
  return dialog;
}

afterEach(() => {
  window.__oveMock!.crashReports = [];
  window.__oveMock!.feedbackUrl = null;
  Reflect.deleteProperty(navigator, "clipboard");
});

describe("buildReport", () => {
  it("puts the description first, then the environment", () => {
    expect(buildReport("  Export froze  ", "startup: app 0.1.0")).toBe(
      "Export froze\n\n--- Environment ---\nstartup: app 0.1.0\n",
    );
  });

  it("says when there is no description and adds a crash report when given", () => {
    const r = buildReport("", "env", { at: 0, text: "Message: boom\n" });
    expect(r).toContain("(no description)");
    expect(r).toContain("--- Latest crash report ---\nMessage: boom");
    expect(
      buildReport("x", "env", { at: CRASH.at, text: "t" }),
    ).toContain("--- Latest crash report (2023-11-14T22:13:20.000Z) ---");
  });
});

describe("What's new", () => {
  it("opens from Changelog on Home with this release's highlights and limitations", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("nav-changelog"));
    const dialog = await screen.findByTestId("whats-new");
    expect(within(dialog).getByRole("dialog").getAttribute("aria-label")).toBe(
      "What's new",
    );
    expect(dialog.textContent).toContain(RELEASE_NOTES.name);
    const highlights = within(
      screen.getByTestId("whats-new-highlights"),
    ).getAllByRole("listitem");
    expect(highlights.length).toBe(RELEASE_NOTES.highlights.length);
    expect(dialog.textContent).toContain("History");
    expect(dialog.textContent).toContain("21 styles");
    const limits = screen.getByTestId("whats-new-limitations").textContent;
    expect(limits).toContain("540p");
    expect(limits).toContain("H.264 only");
    expect(limits).toContain("loudness normalisation");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("whats-new")).toBeNull();
    fireEvent.click(screen.getByTestId("nav-changelog"));
    fireEvent.click(
      within(await screen.findByTestId("whats-new")).getByLabelText("Close"),
    );
    expect(screen.queryByTestId("whats-new")).toBeNull();
  });
});

describe("Send feedback", () => {
  it("copies the description with the environment and sends nothing", async () => {
    const writeText = stubClipboard();
    await openFeedback();
    fireEvent.change(screen.getByTestId("feedback-text"), {
      target: { value: "The export stopped at 40 %." },
    });
    expect(screen.getByTestId("feedback-preview").textContent).toContain(
      "The export stopped at 40 %.",
    );
    fireEvent.click(screen.getByTestId("feedback-copy"));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const copied = (writeText.mock.calls[0] as unknown as [string])[0];
    expect(copied).toContain("The export stopped at 40 %.");
    expect(copied).toContain("--- Environment ---\nstartup: app dev");
    expect(copied).toContain("GPU mock");
    expect(copied).not.toContain("crash report");
    await waitFor(() =>
      expect(screen.getByTestId("feedback-copy").textContent).toContain(
        "Copied",
      ),
    );
    // Nothing leaves the app, and there is no feedback page in this build.
    expect(
      window.__oveMock!.calls.some((c) => c.startsWith("openUrl")),
    ).toBe(false);
    expect(screen.queryByTestId("feedback-open-page")).toBeNull();
  });

  it("leaves the crash report out until the person opts in", async () => {
    window.__oveMock!.crashReports = [CRASH];
    await openFeedback();
    expect(screen.getByTestId("feedback-crash-hint").textContent).toContain(
      "Settings → Privacy",
    );
    expect(screen.queryByTestId("feedback-attach-crash")).toBeNull();
    expect(screen.getByTestId("feedback-preview").textContent).not.toContain(
      CRASH.path,
    );
  });

  it("includes the latest crash report when opted in, and can leave it out", async () => {
    localStorage.setItem(CRASH_OPT_IN_KEY, "on");
    window.__oveMock!.crashReports = [
      CRASH,
      { ...CRASH, path: "/logs/crash-reports/crash-1.txt", at: 1 },
    ];
    const writeText = stubClipboard();
    await openFeedback();
    const attach = screen.getByTestId<HTMLInputElement>(
      "feedback-attach-crash",
    );
    // The newest report (the mock's text is "report <path>").
    await waitFor(() =>
      expect(screen.getByTestId("feedback-preview").textContent).toContain(
        `report ${CRASH.path}`,
      ),
    );
    expect(attach.checked).toBe(true);
    fireEvent.click(screen.getByTestId("feedback-copy"));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect((writeText.mock.calls[0] as unknown as [string])[0]).toContain(
      "--- Latest crash report",
    );
    fireEvent.click(attach);
    expect(screen.getByTestId("feedback-preview").textContent).not.toContain(
      "crash-reports",
    );
  });

  it("opens the build's feedback page in the browser when one is set", async () => {
    window.__oveMock!.feedbackUrl = "https://example.invalid/feedback";
    await openFeedback();
    fireEvent.click(await screen.findByTestId("feedback-open-page"));
    await waitFor(() =>
      expect(window.__oveMock!.calls).toContain(
        "openUrl:https://example.invalid/feedback",
      ),
    );
  });

  it("says so when the clipboard can't be used", async () => {
    await openFeedback();
    fireEvent.click(screen.getByTestId("feedback-copy"));
    const toast = await screen.findByTestId("toast");
    expect(toast.textContent).toContain("Couldn't copy the report");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("feedback-dialog")).toBeNull();
  });
});
