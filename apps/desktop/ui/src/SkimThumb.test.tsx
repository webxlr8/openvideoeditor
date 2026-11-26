// Hover-skim: the pointer picks a tile of the skim sheet in proportion to its position, with no call to the core
// while skimming and the tile in place within a frame (16 ms) of the pointer move.

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { SkimView } from "./bindings";
import { SkimThumb, tileAt, tileStyle } from "./SkimThumb";

const sheet: SkimView = {
  contentId: "0123456789abcdef0123456789abcdef",
  columns: 10,
  rows: 6,
  tileWidth: 160,
  tileHeight: 90,
  count: 60,
};

/** The thumbnail is 200 px wide at x = 100. */
function sized(el: HTMLElement) {
  el.getBoundingClientRect = () =>
    ({ left: 100, top: 0, width: 200, height: 125, right: 300, bottom: 125, x: 100, y: 0 }) as DOMRect;
}

function calls(): string[] {
  return (window as unknown as { __oveMock: { calls: string[] } }).__oveMock.calls;
}

describe("hover-skim", () => {
  it("maps the pointer to tiles from start to end", () => {
    expect(tileAt(0, 60)).toBe(0);
    expect(tileAt(0.5, 60)).toBe(30);
    expect(tileAt(0.999, 60)).toBe(59);
    expect(tileAt(1.2, 60)).toBe(59);
    expect(tileAt(-0.1, 60)).toBe(0);
    expect(tileStyle(sheet, 0).backgroundPosition).toBe("0% 0%");
    expect(tileStyle(sheet, 59).backgroundPosition).toBe("100% 100%");
    expect(tileStyle(sheet, 12).backgroundPosition).toBe(`${(2 / 9) * 100}% 20%`);
    expect(tileStyle(sheet, 0).backgroundSize).toBe("1000% 600%");
  });

  it("follows the pointer without calling the core, each tile within 16 ms", () => {
    const request = vi.fn();
    render(<SkimThumb thumbnail="data:image/png;base64,AA==" skim={sheet} video onRequest={request} />);
    const box = screen.getByTestId("skim-thumb");
    const layer = screen.getByTestId("skim-layer");
    sized(box);
    expect(layer.style.backgroundImage).toContain(`${sheet.contentId}.jpg`);
    expect(layer.style.visibility).toBe("hidden");
    const before = calls().length;
    fireEvent.mouseEnter(box);
    let worst = 0;
    const seen: string[] = [];
    for (let x = 103; x < 300; x += 4) {
      const t = performance.now();
      fireEvent.mouseMove(box, { clientX: x, clientY: 50 });
      worst = Math.max(worst, performance.now() - t);
      seen.push(layer.dataset.tile ?? "");
    }
    expect(worst).toBeLessThanOrEqual(16);
    expect(seen[0]).toBe("0");
    expect(seen[seen.length - 1]).toBe("59");
    // Frames go from start to end, never back.
    const tiles = seen.map(Number);
    expect(tiles.every((t, i) => i === 0 || t >= tiles[i - 1])).toBe(true);
    expect(new Set(tiles).size).toBe(50);
    expect(layer.style.visibility).toBe("visible");
    expect(request).not.toHaveBeenCalled();
    expect(calls().length).toBe(before);
    fireEvent.mouseLeave(box);
    expect(layer.style.visibility).toBe("hidden");
  });

  it("keeps the normal thumbnail and asks once per hover while the sheet is not built", () => {
    const request = vi.fn();
    render(<SkimThumb thumbnail="data:image/png;base64,AA==" skim={null} video onRequest={request} />);
    const box = screen.getByTestId("skim-thumb");
    sized(box);
    expect(screen.queryByTestId("skim-layer")).toBeNull();
    expect(box.querySelector("img")).not.toBeNull();
    fireEvent.mouseEnter(box);
    for (let x = 100; x < 300; x += 20) fireEvent.mouseMove(box, { clientX: x });
    expect(request).toHaveBeenCalledTimes(1);
    fireEvent.mouseLeave(box);
    fireEvent.mouseEnter(box);
    expect(request).toHaveBeenCalledTimes(2);
  });

  it("does not skim or ask for audio files", () => {
    const request = vi.fn();
    render(<SkimThumb thumbnail="data:image/png;base64,AA==" skim={null} video={false} onRequest={request} />);
    fireEvent.mouseEnter(screen.getByTestId("skim-thumb"));
    expect(request).not.toHaveBeenCalled();
  });
});
