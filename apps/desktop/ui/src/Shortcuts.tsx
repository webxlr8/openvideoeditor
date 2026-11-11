// Keyboard shortcuts overlay (press ? in the editor).

const MOD = typeof navigator !== "undefined" && /mac/i.test(navigator.platform) ? "⌘" : "Ctrl";

export const SHORTCUTS: { group: string; keys: [string, string][] }[] = [
  {
    group: "Playback",
    keys: [
      ["Space", "Play / pause"],
      ["J / K / L", "Reverse, pause, forward (repeat for 2×)"],
      ["← / →", "Previous / next frame (Shift: 10 frames)"],
      ["Home / End", "Go to start / end"],
    ],
  },
  {
    group: "Editing",
    keys: [
      ["S", "Split the selected clip at the playhead"],
      ["D", "Duplicate selection"],
      ["Delete", "Delete selection"],
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
      ["+ / −", "Zoom in / out"],
      [`${MOD} 0`, "Fit the timeline"],
      ["Drag an edge", "Trim (the start edge moves the in-point)"],
      ["Right-click", "Clip and lane actions"],
    ],
  },
  { group: "App", keys: [["?", "Show this list"], [`${MOD} ⇧ D`, "Playback diagnostics"]] },
];

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
          {SHORTCUTS.map((g) => (
            <section key={g.group}>
              <h4>{g.group}</h4>
              {g.keys.map(([k, what]) => (
                <div className="shortcut-row" key={k}>
                  <kbd>{k}</kbd>
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
