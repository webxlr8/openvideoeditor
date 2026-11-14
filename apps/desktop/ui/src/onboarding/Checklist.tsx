// The guided first edit as one row along the bottom of the window: five steps and how to do the next one. A row of
// its own covers nothing (a floating card would sit on the timeline or the viewer). It can be closed; Help →
// Getting started brings it back with the same steps ticked (the native side keeps the ticks, see useOnboarding).

import type { ChecklistStep } from "../bindings";
import { Icon } from "../icons";
import { STEPS } from "./checklist";
import "./onboarding.css";

export function Checklist({
  done,
  onClose,
}: {
  done: ChecklistStep[];
  onClose: () => void;
}) {
  const count = STEPS.filter((s) => done.includes(s.id)).length;
  const next = STEPS.find((s) => !done.includes(s.id));
  return (
    <aside
      className="checklist"
      aria-label="Getting started"
      data-testid="checklist"
    >
      <strong>Getting started</strong>
      <ol>
        {STEPS.map((s, i) => {
          const ticked = done.includes(s.id);
          return (
            <li
              key={s.id}
              className={ticked ? "done" : s === next ? "next" : ""}
              title={s.how}
              data-testid={`checklist-step-${s.id}`}
              data-done={ticked}
            >
              <span className="checklist-mark" aria-hidden>
                {ticked ? <Icon name="check" size={11} /> : i + 1}
              </span>
              {s.label}
              {ticked && <span className="sr-only"> (done)</span>}
            </li>
          );
        })}
      </ol>
      <span className="checklist-next muted" data-testid="checklist-next">
        {next
          ? `Next: ${next.how}`
          : "All five done. That was a whole edit."}
      </span>
      <span className="muted small" data-testid="checklist-count">
        {count} of {STEPS.length}
      </span>
      <button
        className="icon-btn"
        title="Close (Help → Getting started brings it back)"
        aria-label="Close getting started"
        onClick={onClose}
        data-testid="checklist-close"
      >
        <Icon name="close" size={14} />
      </button>
    </aside>
  );
}
