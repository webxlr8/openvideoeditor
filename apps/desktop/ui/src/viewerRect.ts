import { useEffect, useRef } from "react";
import type { ViewerRect } from "./bindings";

/** Measures the viewer element relative to the web view, in CSS pixels. */
export function measureViewerRect(el: HTMLElement | null, visible: boolean): ViewerRect {
  const scaleFactor = window.devicePixelRatio || 1;
  if (!el || !visible || !el.isConnected) {
    return { x: 0, y: 0, width: 0, height: 0, scaleFactor, visible: false };
  }
  const r = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  const shown = style.display !== "none" && style.visibility !== "hidden" && r.width >= 1 && r.height >= 1;
  return { x: r.left, y: r.top, width: r.width, height: r.height, scaleFactor, visible: shown };
}

export function sameRect(a: ViewerRect | null, b: ViewerRect): boolean {
  return (
    !!a &&
    a.visible === b.visible &&
    a.scaleFactor === b.scaleFactor &&
    a.x === b.x &&
    a.y === b.y &&
    a.width === b.width &&
    a.height === b.height
  );
}

/**
 * Keeps the native viewer aligned with `ref`: re-measures on element resize, window resize,
 * display scale changes and every animation frame (position can change without a resize, e.g.
 * when a side panel collapses), and sends the rectangle only when it changed.
 */
export function useViewerRectSync(
  ref: React.RefObject<HTMLElement | null>,
  visible: boolean,
  send: (rect: ViewerRect) => void,
) {
  const last = useRef<ViewerRect | null>(null);
  const sendRef = useRef(send);
  useEffect(() => {
    sendRef.current = send;
  }, [send]);

  useEffect(() => {
    let raf = 0;
    const check = () => {
      const rect = measureViewerRect(ref.current, visible);
      if (!sameRect(last.current, rect)) {
        last.current = rect;
        sendRef.current(rect);
      }
    };
    const loop = () => {
      check();
      raf = requestAnimationFrame(loop);
    };
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(check) : null;
    if (ref.current && ro) ro.observe(ref.current);
    window.addEventListener("resize", check);
    let mq: MediaQueryList | null = null;
    const watchScale = () => {
      mq?.removeEventListener("change", onScale);
      mq = window.matchMedia?.(`(resolution: ${window.devicePixelRatio || 1}dppx)`) ?? null;
      mq?.addEventListener("change", onScale);
    };
    const onScale = () => {
      check();
      watchScale();
    };
    watchScale();
    loop();
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
      window.removeEventListener("resize", check);
      mq?.removeEventListener("change", onScale);
      // Hide the native surface when the viewer unmounts.
      const hidden = measureViewerRect(null, false);
      last.current = hidden;
      sendRef.current(hidden);
    };
  }, [ref, visible]);
}
