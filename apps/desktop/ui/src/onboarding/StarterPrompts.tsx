// What an empty chat offers: three example requests and an answer to "What can you do?". Tapping an example puts
// its words in the message box for the person to finish or send; nothing is sent by the tap.

import { useState } from "react";
import "./onboarding.css";

/** Example requests. One ends open, for the person's own words. */
export const STARTERS: string[] = [
  "Cut the pauses out of the main track",
  "Make captions for everything that is said",
  "Add a title at the start that says ",
];

export function StarterPrompts({ onPick }: { onPick: (text: string) => void }) {
  const [answer, setAnswer] = useState(false);
  return (
    <div className="starters" data-testid="starters">
      {STARTERS.map((s, i) => (
        <button
          key={s}
          className="chip starter"
          title="Puts this in the message box. Nothing is sent until you send it."
          data-testid={`starter-${i + 1}`}
          onClick={() => onPick(s)}
        >
          {s.trim()}
          {s.endsWith(" ") ? " …" : ""}
        </button>
      ))}
      <button
        className="starter-what"
        aria-expanded={answer}
        data-testid="starter-what"
        onClick={() => setAnswer((a) => !a)}
      >
        What can you do?
      </button>
      {answer && (
        <p className="starter-answer small" data-testid="starter-answer">
          I can cut, trim and move clips, add titles, captions, transitions
          and effects, set sound levels, and get the project ready to export.
          To do that I look at this project's timeline, frames of its video,
          its sound levels and its transcripts. Every change I make is one undo
          step, and I work on this project only.
        </p>
      )}
    </div>
  );
}
