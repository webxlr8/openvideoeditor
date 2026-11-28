// Keyboard shortcuts overlay (press ? in the editor). Keys rebound in Settings → Shortcuts show as bound.

import { MOD, REBINDABLE, bindingOf, keyLabel } from "./keys";

/** How default keys read best ("=" also answers to "+"). */
const DEFAULT_LABEL: Record<string, string> = { zoomIn: "+", zoomOut: "−" };

/** A rebindable action's key as it is bound now. */
const k = (id: string) => {
  const key = bindingOf(id);
  const isDefault = REBINDABLE.find((r) => r.id === id)?.key === key;
  return (isDefault && DEFAULT_LABEL[id]) || keyLabel(key);
};

export function shortcutGroups(): { group: string; keys: [string, string][] }[] {
  return [
    {
      group: "Playback",
      keys: [
        [k("togglePlay"), "Play / pause"],
        [
          `${k("shuttleReverse")} / ${k("pause")} / ${k("shuttleForward")}`,
          "Reverse, pause, forward (repeat for 2×)",
        ],
        ["← / →", "Previous / next frame (Shift: 10 frames)"],
        ["Home / End", "Go to start / end"],
      ],
    },
    {
      group: "Editing",
      keys: [
        [k("split"), "Split the selected clip at the playhead"],
        [k("duplicate"), "Duplicate selection"],
        ["Delete", "Delete selection (linked items too)"],
        ["⇧ Delete", "Ripple delete: the selection, or the marked range"],
        [`${k("rippleTrimStart")} / ${k("rippleTrimEnd")}`, "Ripple trim the start / end to the playhead"],
        [`${k("insert")} / ${k("overwrite")}`, "Insert / overwrite with the source clip at the playhead"],
        [`${k("markIn")} / ${k("markOut")} / ${k("markClip")}`, "Mark in / out / the selected clip"],
        [`${MOD} C / ${MOD} V`, "Copy / paste at the playhead"],
        [`${MOD} A`, "Select all on the track"],
        [`${MOD} Z / ⇧${MOD} Z`, "Undo / redo"],
        ["Shift-click", "Add to selection"],
        ["Esc", "Clear selection"],
      ],
    },
    {
      group: "Timeline",
      keys: [
        [`${k("zoomIn")} / ${k("zoomOut")}`, "Zoom in / out"],
        [`${MOD} 0`, "Fit the timeline"],
        [`${k("toolSelect")} / ${k("toolBlade")} / ${k("toolTrim")}`, "Select, blade and trim tools"],
        ["Drag an edge", "Trim (the start edge moves the in-point; Alt: this side only)"],
        ["Trim tool", "Drag a clip to slip, Alt-drag to slide, drag an edit point to roll"],
        ["Right-click", "Clip and lane actions"],
      ],
    },
    {
      group: "App",
      keys: [
        [k("help"), "Show this list"],
        [`${MOD} ⇧ D`, "Playback diagnostics"],
      ],
    },
    {
      group: "Assistant",
      keys: [
        ["@", "Mention a media file, a clip or the marked range: the assistant gets its exact timeline lines and can edit it at once"],
        ["/", "At the start of a message: pick a skill for the assistant to follow, or a template"],
        ["↑ / ↓, Enter", "Move in the @ or / list, pick (Esc closes it)"],
      ],
    },
  ];
}

export function Shortcuts({ onClose }: { onClose: () => void }) {
  return (
    <div className="modal-backdrop" onMouseDown={onClose} data-testid="shortcuts-overlay">
      <div className="modal shortcuts" role="dialog" aria-label="Keyboard shortcuts" onMouseDown={(e) => e.stopPropagation()}>
        <header>
          <h3>Keyboard shortcuts</h3>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>
        <div className="shortcut-groups">
          {shortcutGroups().map((g) => (
            <section key={g.group}>
              <h4>{g.group}</h4>
              {g.keys.map(([key, what]) => (
                <div className="shortcut-row" key={what}>
                  <kbd>{key}</kbd>
                  <span>{what}</span>
                </div>
              ))}
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
