// A media thumbnail that skims: moving the pointer across it shows frames from start to end, taken from the file's
// skim sheet (one image, tiles in a grid). Skimming only moves a CSS background: no decoding and no call to the
// core per pointer move. When the sheet is not built yet, the normal thumbnail stays and the core is asked once per
// hover to build it sooner.

import { useRef } from "react";
import type { SkimView } from "./bindings";
import { skimUrl } from "./bridge";
import "./skim.css";

/** The tile for a pointer at `fraction` (0..1) across the thumbnail. */
export function tileAt(fraction: number, count: number): number {
  if (count <= 0) return 0;
  return Math.min(count - 1, Math.max(0, Math.floor(fraction * count)));
}

/** CSS `background-position` and `background-size` that show tile `i` of a sheet in a box of any size. */
export function tileStyle(
  skim: SkimView,
  i: number,
): { backgroundPosition: string; backgroundSize: string } {
  const col = i % skim.columns;
  const row = Math.floor(i / skim.columns);
  // Percent positions: (index / (cells − 1)) moves the sheet so that cell lines up with the box.
  const x = skim.columns > 1 ? (col / (skim.columns - 1)) * 100 : 0;
  const y = skim.rows > 1 ? (row / (skim.rows - 1)) * 100 : 0;
  return {
    backgroundPosition: `${x}% ${y}%`,
    backgroundSize: `${skim.columns * 100}% ${skim.rows * 100}%`,
  };
}

export function SkimThumb({
  thumbnail,
  skim,
  video,
  onRequest,
}: {
  thumbnail: string | null;
  skim: SkimView | null | undefined;
  /** Only video files skim. */
  video: boolean;
  /** Asks the core to build the sheet sooner (once per hover, when it is missing). */
  onRequest: () => void;
}) {
  const layer = useRef<HTMLDivElement>(null);
  const asked = useRef(false);

  const show = (e: React.MouseEvent<HTMLDivElement>) => {
    const el = layer.current;
    if (!skim || !el) return;
    const r = e.currentTarget.getBoundingClientRect();
    const fraction = r.width > 0 ? (e.clientX - r.left) / r.width : 0;
    const i = tileAt(fraction, skim.count);
    if (el.dataset.tile === String(i)) return;
    // Straight to the element: no React render between the pointer and the frame.
    const s = tileStyle(skim, i);
    el.style.backgroundPosition = s.backgroundPosition;
    el.style.backgroundSize = s.backgroundSize;
    el.style.visibility = "visible";
    el.dataset.tile = String(i);
  };

  const hide = () => {
    asked.current = false;
    const el = layer.current;
    if (!el) return;
    el.style.visibility = "hidden";
    delete el.dataset.tile;
  };

  const enter = () => {
    if (video && !skim && !asked.current) {
      asked.current = true;
      onRequest();
    }
  };

  return (
    <div
      className="skim-thumb"
      onMouseEnter={enter}
      onMouseMove={show}
      onMouseLeave={hide}
      data-testid="skim-thumb"
    >
      {thumbnail && <img src={thumbnail} alt="" />}
      {skim && (
        <div
          ref={layer}
          className="skim-layer"
          data-testid="skim-layer"
          style={{ backgroundImage: `url("${skimUrl(skim.contentId)}")`, visibility: "hidden" }}
        />
      )}
    </div>
  );
}
