// Guides, the full-screen viewer and playback rates against the browser mock (add-viewer-interaction-and-text-fills
// tasks 3.1, 3.3 and 3.4). The core's side is tested in Rust: rates in crates/ove-playback/tests/player.rs, the
// control-link refusal in apps/desktop/src-tauri/src/control.rs, agent overlays in crates/ove-tools/tests.

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { MediaInfo_Serialize, PlaybackState, ProjectView_Serialize, TransportCommand } from "../bindings";
import { commands } from "../bridge";
import { fire } from "../notify";
import { Viewer } from "../Viewer";
import { HIDE_AFTER_MS } from "./FullscreenControls";
import { maskRect } from "./guides";

const MEDIA: MediaInfo_Serialize = {
  path: "/mock/p.ove",
  name: "p",
  durationSeconds: 5,
  width: 1920,
  height: 1080,
  frameRate: 30,
  totalFrames: 150,
  videoCodec: "timeline",
  audioLayout: null,
  placeholder: false,
};

const PLAYBACK: PlaybackState = {
  hasMedia: true,
  frame: 30,
  totalFrames: 150,
  frameRate: 30,
  playing: false,
  speed: 0,
  looping: false,
  volume: 1,
  muted: false,
  audioAvailable: false,
  rate: 1,
};

beforeAll(() => {
  // jsdom has no layout: give the viewer area a size.
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 848 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 498 });
});

afterEach(() => {
  vi.useRealTimers();
  localStorage.clear();
});

async function project(): Promise<ProjectView_Serialize> {
  const r = await commands.createProject("Chrome", null);
  if (r.status !== "ok" || !r.data) throw new Error("no project");
  return r.data;
}

function Harness({ p }: { p: ProjectView_Serialize }) {
  const [playback, setPlayback] = useState(PLAYBACK);
  const onTransport = (c: TransportCommand) => {
    fire(
      commands.transport(c).then((r) => {
        if (r.status === "ok") setPlayback({ ...PLAYBACK, rate: r.data.rate });
      }),
    );
  };
  return (
    <Viewer
      media={MEDIA}
      frame={30}
      backend="stream"
      visible
      diagnosticsOpen={false}
      onError={(e) => {
        throw new Error(e.message);
      }}
      transport={{ playback, onTransport }}
      overlay={{ project: p, time: 1, playing: false, selection: [], onSelect: () => {}, onEdit: () => {} }}
    />
  );
}

const calls = () => window.__oveMock!.calls;

describe("viewer guides", () => {
  it("draws grids, safe areas, the centre and a mask as view state only", async () => {
    const p = await project();
    const rev = p.rev;
    render(<Harness p={p} />);
    fireEvent.click(screen.getByTestId("guides-button"));
    fireEvent.click(screen.getByTestId("guides-thirds"));
    fireEvent.click(screen.getByTestId("guides-title"));
    fireEvent.click(screen.getByTestId("guides-action"));
    fireEvent.click(screen.getByTestId("guides-centre"));
    fireEvent.change(screen.getByTestId("guides-grid"), { target: { value: "4" } });
    fireEvent.change(screen.getByTestId("guides-mask"), { target: { value: "9:16" } });
    const layer = await screen.findByTestId("guide-layer");
    // The viewer box is 800×450 (848×498 less the padding): title safe is inset 5 %, action safe 3.5 %.
    const title = screen.getByTestId("guide-title");
    expect([title.getAttribute("x"), title.getAttribute("y"), title.getAttribute("width")]).toEqual(["40", "22.5", "720"]);
    expect(Number(screen.getByTestId("guide-action").getAttribute("x"))).toBeCloseTo(28, 5);
    expect(layer.querySelectorAll(".guide-thirds").length).toBe(4);
    expect(layer.querySelectorAll(".guide-grid").length).toBe(6);
    expect(screen.getByTestId("guide-centre")).toBeTruthy();
    expect(screen.getByTestId("guide-mask").getAttribute("data-mask")).toBe("9:16");
    // The guides are this project's view state: stored in the browser, not sent to the document.
    expect(JSON.parse(localStorage.getItem(`ove.guides.${p.id}`)!)).toMatchObject({ thirds: true, grid: 4, mask: "9:16" });
    const v = await commands.projectView();
    expect(v.status === "ok" && v.data?.rev).toBe(rev);
    expect(calls().some((c) => c.startsWith("updateItem") || c.startsWith("viewerPreview"))).toBe(false);
    // All off.
    fireEvent.click(screen.getByTestId("guides-off"));
    await waitFor(() => expect(screen.queryByTestId("guide-layer")).toBeNull());
  });

  it("masks keep the centred part of the frame", () => {
    expect(maskRect("2.39:1", 1920, 1080)).toEqual({ x: 0, y: (1080 - 1920 / 2.39) / 2, w: 1920, h: 1920 / 2.39 });
    expect(maskRect("1:1", 1920, 1080)).toEqual({ x: 420, y: 0, w: 1080, h: 1080 });
    expect(maskRect("9:16", 1080, 1920)).toEqual({ x: 0, y: 0, w: 1080, h: 1920 });
  });

  it("edits a platform zone preset and shows it", async () => {
    const p = await project();
    render(<Harness p={p} />);
    fireEvent.click(screen.getByTestId("guides-button"));
    fireEvent.click(screen.getByTestId("guides-edit-presets"));
    fireEvent.change(screen.getByTestId("guides-preset-top-0"), { target: { value: "20" } });
    fireEvent.change(screen.getByTestId("guides-platform"), { target: { value: "vertical-feed" } });
    const zone = await screen.findByTestId("guide-platform");
    expect(zone.getAttribute("y")).toBe("90");
    const stored = JSON.parse(localStorage.getItem("ove.guidePresets")!) as { top: number }[];
    expect(stored[0].top).toBe(20);
  });
});

describe("full-screen viewer", () => {
  it("fills the window, hides its controls after 2 s without movement, and Esc returns", async () => {
    const p = await project();
    render(<Harness p={p} />);
    vi.useFakeTimers();
    fireEvent.click(screen.getByTestId("viewer-fullscreen"));
    const area = screen.getByTestId("viewer-area");
    expect(area.getAttribute("data-full")).toBe("true");
    expect(area.className).toContain("is-full");
    expect(calls()).toContain("viewerFullscreen:true");
    const controls = screen.getByTestId("fullscreen-controls");
    expect(controls.getAttribute("data-shown")).toBe("true");
    act(() => {
      vi.advanceTimersByTime(HIDE_AFTER_MS + 50);
    });
    expect(screen.getByTestId("fullscreen-controls").getAttribute("data-shown")).toBe("false");
    fireEvent.pointerMove(area, { clientX: 10, clientY: 10 });
    expect(screen.getByTestId("fullscreen-controls").getAttribute("data-shown")).toBe("true");
    // The floating controls carry play, scrub, timecode, loop, rate and guides.
    for (const id of ["fs-play", "fs-scrub", "fs-timecode", "fs-loop", "fs-rate-button", "fs-guides-button"]) {
      expect(screen.getByTestId(id)).toBeTruthy();
    }
    expect(screen.getByTestId("fs-timecode").textContent).toBe("00:00:01:00");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(area.getAttribute("data-full")).toBe("false");
    expect(screen.queryByTestId("fullscreen-controls")).toBeNull();
    expect(calls()).toContain("viewerFullscreen:false");
  });
});

describe("playback rates", () => {
  it("offers 0.5× to 2× and refuses other rates", async () => {
    const p = await project();
    render(<Harness p={p} />);
    fireEvent.click(screen.getByTestId("viewer-fullscreen"));
    fireEvent.click(screen.getByTestId("fs-rate-button"));
    const items = screen.getByTestId("fs-rate-menu").querySelectorAll("[role=menuitem]");
    expect([...items].map((i) => i.textContent)).toEqual(["0.5×", "✓ 1×", "1.25×", "1.5×", "2×"]);
    fireEvent.click(screen.getByTestId("fs-rate-item-1.5"));
    await waitFor(() => expect(screen.getByTestId("fs-rate-button").textContent).toContain("1.5×"));
    expect(calls()).toContain("transport:setRate:1.5");
    const r = await commands.transport({ kind: "setRate", rate: 7 });
    expect(r.status).toBe("error");
    expect(r.status === "error" && r.error.message).toContain("supported rates: 0.5, 1, 1.25, 1.5, 2");
  });
});
