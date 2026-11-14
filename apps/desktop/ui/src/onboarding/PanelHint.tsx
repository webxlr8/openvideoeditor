// What an empty panel shows: one sentence saying what the panel is for, and one action that gets it started.

import type { ReactNode } from "react";
import type { ItemView_Serialize, ProjectView_Serialize } from "../bindings";
import "./onboarding.css";

export function PanelHint({
  panel,
  children,
  action,
  onAction,
}: {
  /** The panel's id on the editor's rail (names the hint for tests). */
  panel: string;
  /** The one sentence. */
  children: ReactNode;
  /** The action's label; without `onAction` the hint is the sentence alone. */
  action?: string;
  onAction?: () => void;
}) {
  return (
    <div className="panel-hint" data-testid={`hint-${panel}`}>
      <p>{children}</p>
      {action && onAction && (
        <button
          className="btn"
          onClick={onAction}
          data-testid={`hint-${panel}-action`}
        >
          {action}
        </button>
      )}
    </div>
  );
}

/**
 * The action of a panel that works on the selected clip, when nothing is selected: select the first clip the panel
 * can work on, or, in a project without one, import media.
 */
export function selectFirst(
  project: ProjectView_Serialize,
  fits: (i: ItemView_Serialize) => boolean,
  onSelect: (i: ItemView_Serialize) => void,
  onImport?: () => void,
): { action?: string; onAction?: () => void } {
  const first = project.items
    .filter(fits)
    .sort((a, b) => (a.start ?? 0) - (b.start ?? 0))[0];
  if (first) return { action: "Select the first clip", onAction: () => onSelect(first) };
  return { action: "Import media", onAction: onImport };
}
