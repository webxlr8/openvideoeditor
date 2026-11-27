// Floating controls of the full-screen viewer (add-viewer-interaction-and-text-fills task 3.3): play, scrub, timecode,
// loop, rate and guides. They hide after 2 s without pointer movement and come back when the pointer moves.

import { type ReactNode, useEffect, useState } from "react";
import type { PlaybackState, TransportCommand } from "../bindings";
import { Icon } from "../icons";
import { RateMenu } from "./RateMenu";

/** Time without pointer movement after which the controls hide (design D5). */
export const HIDE_AFTER_MS = 2000;

function timecode(frame: number, fps: number): string {
  const f = Math.max(0, Math.round(fps || 30));
  const total = Math.floor(frame / f);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(total / 3600))}:${pad(Math.floor(total / 60) % 60)}:${pad(total % 60)}:${pad(frame % f)}`;
}

export function FullscreenControls({
  playback,
  onTransport,
  onExit,
  guidesMenu,
  activity,
}: {
  playback: PlaybackState;
  onTransport: (c: TransportCommand) => void;
  onExit: () => void;
  guidesMenu: ReactNode;
  /** Changes whenever the pointer moves over the viewer. */
  activity: number;
}) {
  const [shown, setShown] = useState(true);
  useEffect(() => {
    const t = setTimeout(() => setShown(false), HIDE_AFTER_MS);
    return () => {
      clearTimeout(t);
      setShown(true);
    };
  }, [activity]);
  const last = Math.max(0, playback.totalFrames - 1);
  return (
    <div
      className={`fullscreen-controls${shown ? "" : " hidden"}`}
      data-testid="fullscreen-controls"
      data-shown={shown ? "true" : "false"}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <button
        className="icon-btn play"
        data-testid="fs-play"
        aria-label={playback.playing ? "Pause" : "Play"}
        onClick={() => onTransport({ kind: "togglePlay" })}
      >
        <Icon name={playback.playing ? "pause" : "play"} size={18} />
      </button>
      <input
        className="fs-scrub"
        data-testid="fs-scrub"
        type="range"
        min={0}
        max={last}
        step={1}
        aria-label="Playhead"
        value={Math.min(playback.frame, last)}
        onChange={(e) => onTransport({ kind: "seekFrame", frame: Number(e.target.value) })}
      />
      <span className="fs-timecode" data-testid="fs-timecode">
        {timecode(playback.frame, playback.frameRate ?? 30)}
      </span>
      <button
        className={`icon-btn${playback.looping ? " on" : ""}`}
        data-testid="fs-loop"
        aria-label="Loop"
        aria-pressed={playback.looping}
        onClick={() => onTransport({ kind: "setLoop", enabled: !playback.looping })}
      >
        <Icon name="loop" size={16} />
      </button>
      <RateMenu rate={playback.rate ?? 1} onTransport={onTransport} testId="fs-rate" />
      {guidesMenu}
      <button className="icon-btn" data-testid="fs-exit" title="Leave full screen (Esc)" onClick={onExit}>
        <Icon name="close" size={16} />
      </button>
    </div>
  );
}
