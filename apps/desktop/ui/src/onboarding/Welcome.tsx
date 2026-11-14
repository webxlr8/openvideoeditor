// The welcome: three screens before Home on a first launch (what the app does, pick an assistant, privacy), each
// with Skip. It never asks for a key. Help → "Show the welcome again" opens the same thing later.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AssistantStep } from "./AssistantStep";
import "./onboarding.css";

const SCREENS = ["what", "assistant", "privacy"] as const;

export function Welcome({
  assistant: initial,
  onFinish,
  onSkip,
}: {
  /** The assistant chosen earlier, when the welcome is shown again. */
  assistant?: string | null;
  /** The last screen was reached and left: with the chosen assistant, and whether to open the sample project. */
  onFinish: (assistant: string | null, sample: boolean) => void;
  onSkip: () => void;
}) {
  const [step, setStep] = useState(0);
  const [assistant, setAssistant] = useState<string | null>(initial ?? null);
  const root = useRef<HTMLDivElement>(null);
  const primary = useRef<HTMLButtonElement>(null);
  const screen = SCREENS[step];
  const last = step === SCREENS.length - 1;

  // Enter moves on from each screen.
  useEffect(() => primary.current?.focus(), [step]);

  // The window under the welcome (Home, or the editor when it is shown again) must not act on keys meant for it:
  // keys pressed with the focus outside the welcome stop here. Escape skips. In place as soon as the welcome is on
  // screen (a layout effect), so no key slips through before it.
  useLayoutEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (root.current?.contains(e.target as Node)) return;
      e.stopPropagation();
      if (e.key === "Escape") onSkip();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onSkip]);

  return (
    <div
      className="welcome"
      role="dialog"
      aria-modal="true"
      aria-label="Welcome"
      data-testid="welcome"
      ref={root}
      onKeyDown={(e) => {
        // Keys pressed inside go no further than the welcome either.
        e.stopPropagation();
        if (e.key === "Escape") onSkip();
      }}
    >
      <div className="welcome-card" data-testid={`welcome-screen-${screen}`}>
        <div className="welcome-progress" data-testid="welcome-progress">
          {SCREENS.map((s, i) => (
            <span key={s} className={i === step ? "on" : ""} />
          ))}
          <span className="muted small">
            {step + 1} of {SCREENS.length}
          </span>
        </div>

        {screen === "what" && (
          <>
            <h1>Edit video on your computer</h1>
            <p>
              OpenVideoEditor cuts clips, adds titles and captions, and exports
              the result. Do it by hand, or tell an assistant what you want.
            </p>
            <ul className="welcome-points">
              <li>
                <strong>Your work is saved as you go.</strong> Every change is
                kept and can be undone.
              </li>
              <li>
                <strong>An assistant can edit for you.</strong> It works on the
                same timeline you do, one undo step at a time.
              </li>
              <li>
                <strong>There is a sample project.</strong> Try things on it
                before you bring your own footage.
              </li>
            </ul>
          </>
        )}

        {screen === "assistant" && (
          <>
            <h1>Pick an assistant</h1>
            <p>
              An assistant edits the timeline when you describe what you want.
              You can change this later in Settings.
            </p>
            <AssistantStep value={assistant} onChange={setAssistant} />
          </>
        )}

        {screen === "privacy" && (
          <>
            <h1>What stays on this computer</h1>
            <ul className="welcome-points" data-testid="welcome-privacy">
              <li>
                <strong>Media files stay on this computer.</strong> So do your
                projects: Settings → Library shows where they are kept.
              </li>
              <li>
                <strong>The assistant you choose sees the project.</strong> The
                timeline, frames of your video, sound measurements and
                transcripts go to that assistant's provider.
              </li>
              <li>
                <strong>Nothing is sent when no assistant is chosen.</strong>{" "}
                <span data-testid="welcome-choice">
                  {assistant === null
                    ? "You chose to edit by hand."
                    : "You chose an assistant."}
                </span>
              </li>
            </ul>
          </>
        )}

        <div className="welcome-actions">
          <button
            type="button"
            className="welcome-skip"
            onClick={onSkip}
            data-testid="welcome-skip"
          >
            Skip
          </button>
          <span className="grow" />
          {step > 0 && (
            <button
              type="button"
              className="btn"
              onClick={() => setStep(step - 1)}
              data-testid="welcome-back"
            >
              Back
            </button>
          )}
          {last ? (
            <>
              <button
                type="button"
                className="btn"
                onClick={() => onFinish(assistant, true)}
                data-testid="welcome-sample"
              >
                Try the sample
              </button>
              <button
                type="button"
                className="btn primary"
                ref={primary}
                onClick={() => onFinish(assistant, false)}
                data-testid="welcome-done"
              >
                Start editing
              </button>
            </>
          ) : (
            <button
              type="button"
              className="btn primary"
              ref={primary}
              onClick={() => setStep(step + 1)}
              data-testid="welcome-next"
            >
              Next
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
