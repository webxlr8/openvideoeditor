// The one Help entry, on Home (a sidebar row) and in the editor (a button in the transport bar): getting started,
// the welcome, the sample project and the keyboard shortcuts, each one step away.

import { useEffect, useRef, useState } from "react";
import { Icon } from "../icons";
import "./onboarding.css";

/** What Help can reopen. The app provides these; without one, its entry shows disabled. */
export interface HelpActions {
  /** Shows the getting-started checklist again. */
  gettingStarted: () => void;
  /** Shows the welcome again. */
  welcome: () => void;
  /** Makes a new copy of the sample project and opens it. */
  sample: () => void;
}

export function HelpMenu({
  actions,
  onShortcuts,
  variant,
}: {
  actions?: HelpActions;
  /** Shows the keyboard shortcuts (Settings → Shortcuts on Home, the list in the editor). */
  onShortcuts: () => void;
  /** A labelled sidebar row, or an icon button. */
  variant: "nav" | "icon";
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);
  const items: { id: string; label: string; run?: () => void }[] = [
    { id: "getting-started", label: "Getting started", run: actions?.gettingStarted },
    { id: "welcome", label: "Show the welcome again", run: actions?.welcome },
    { id: "sample", label: "Try the sample project", run: actions?.sample },
    { id: "shortcuts", label: "Keyboard shortcuts", run: onShortcuts },
  ];
  return (
    <div className={`dropdown help-menu help-${variant}`} ref={root}>
      <button
        className={variant === "nav" ? "nav-item" : `icon-btn${open ? " on" : ""}`}
        title={variant === "icon" ? "Help" : undefined}
        aria-label="Help"
        aria-haspopup="menu"
        aria-expanded={open}
        data-testid="help"
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="help" size={16} />
        {variant === "nav" && " Help"}
      </button>
      {open && (
        <div
          className={`menu menu-up ${variant === "nav" ? "menu-left" : "menu-right"}`}
          role="menu"
          data-testid="help-menu"
        >
          {items.map((item) => (
            <button
              key={item.id}
              role="menuitem"
              className="menu-item"
              data-testid={`help-${item.id}`}
              disabled={!item.run}
              onClick={() => {
                setOpen(false);
                item.run?.();
              }}
            >
              <span>{item.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
