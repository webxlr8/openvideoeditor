// The viewer side of the key colour sampler: while a chroma key's sampler is armed, a crosshair layer over the
// frame takes one click or drag and sends it to the core (see keySamplerStore.ts).

import { useRef, useState } from "react";
import type { AppError } from "./bindings";
import { commands } from "./bridge";
import { sampleRect, stopKeySampling, useKeySampling } from "./keySamplerStore";
import { fire } from "./notify";

export function KeySamplerOverlay({ onError }: { onError: (e: AppError) => void }) {
  const sampling = useKeySampling();
  const [from, setFrom] = useState<{ x: number; y: number } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  if (!sampling) return null;
  const send = async (rect: number[]) => {
    const r = await commands.sampleKeyColor(sampling.itemId, sampling.effectId, sampling.time, rect);
    if (r.status === "error") {
      onError(r.error);
      return;
    }
    if (r.data.outside) {
      setMessage("That point is outside the selected layer. Click on the layer's screen colour.");
      return;
    }
    setMessage(null);
    stopKeySampling();
  };
  const at = (e: React.PointerEvent) => {
    const r = ref.current?.getBoundingClientRect();
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0), w: r?.width ?? 1, h: r?.height ?? 1 };
  };
  return (
    <div
      ref={ref}
      className="key-sampler"
      data-testid="key-sampler"
      style={{ position: "absolute", inset: 0, cursor: "crosshair", zIndex: 5 }}
      onPointerDown={(e) => {
        const p = at(e);
        setFrom({ x: p.x, y: p.y });
      }}
      onPointerUp={(e) => {
        const p = at(e);
        const start = from ?? { x: p.x, y: p.y };
        setFrom(null);
        fire(send(sampleRect(start.x, start.y, p.x, p.y, p.w, p.h)), onError);
      }}
    >

      <div className="key-sampler-hint" data-testid="key-sampler-msg">
        {message ?? "Click or drag on the screen colour to key."}
      </div>
      <button
        className="btn"
        data-testid="key-sampler-cancel"
        style={{ position: "absolute", right: 8, top: 8 }}
        onPointerDown={(e) => e.stopPropagation()}
        onPointerUp={(e) => e.stopPropagation()}
        onClick={() => stopKeySampling()}
      >
        Cancel
      </button>
    </div>
  );
}
