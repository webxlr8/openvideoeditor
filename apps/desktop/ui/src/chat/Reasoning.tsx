// Streamed reasoning (change add-agent-chat-context-and-activity, design D4): a collapsed block with the first line
// and the elapsed time. Expanded, it shows the text so far and keeps updating. Whether it starts open is remembered.

import { useEffect, useState } from "react";
import { Icon } from "../icons";

const OPEN_KEY = "ove.reasoningOpen";

function readOpen(): boolean {
  try {
    return localStorage.getItem(OPEN_KEY) === "1";
  } catch {
    return false;
  }
}

function storeOpen(open: boolean) {
  try {
    localStorage.setItem(OPEN_KEY, open ? "1" : "0");
  } catch {
    // Not remembered; the block still works.
  }
}

/** Seconds as the block shows them. */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

export function Reasoning({
  text,
  started,
  ended,
}: {
  text: string;
  /** When the reasoning started streaming (ms); absent for a reopened chat. */
  started?: number;
  /** When it ended; absent while it streams. */
  ended?: number;
}) {
  const [open, setOpen] = useState(readOpen);
  const [now, setNow] = useState(() => Date.now());
  const live = started !== undefined && ended === undefined;
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [live]);
  const first = text.trim().split("\n")[0] ?? "";
  const time = started !== undefined ? elapsed((ended ?? now) - started) : null;
  return (
    <div className="reasoning" data-testid="reasoning">
      <button
        className="reasoning-head"
        aria-expanded={open}
        onClick={() => {
          storeOpen(!open);
          setOpen(!open);
        }}
        data-testid="reasoning-toggle"
      >
        <Icon name={open ? "chevronDown" : "expand"} size={12} />
        <span data-testid="reasoning-time">
          {live ? `Thinking ${time}` : time ? `Thought for ${time}` : "Reasoning"}
        </span>
        {!open && <span className="first">· {first}</span>}
      </button>
      {open && (
        <div className="reasoning-body" data-testid="reasoning-body">
          {text}
        </div>
      )}
    </div>
  );
}
