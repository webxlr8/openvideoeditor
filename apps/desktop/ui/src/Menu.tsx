import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "./icons";

export interface MenuItem {
  id: string;
  label: string;
  hint?: string;
  danger?: boolean;
  onSelect?: () => void;
}

/** A button that opens a dropdown list. The list is rendered by the web view, so it draws over
 * the native viewer (candidate 1) like any other web content. */
export function Dropdown({
  label,
  items,
  testId,
  align = "right",
  direction = "down",
  icon,
}: {
  label: ReactNode;
  items: MenuItem[];
  testId: string;
  align?: "left" | "right";
  direction?: "down" | "up";
  icon?: ReactNode;
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
  return (
    <div className="dropdown" ref={root}>
      <button
        className="pill"
        data-testid={`${testId}-button`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        {icon}
        {label}
        <Icon name="chevronDown" size={14} />
      </button>
      {open && (
        <div className={`menu menu-${align} menu-${direction}`} role="menu" data-testid={`${testId}-menu`}>
          {items.map((item) => (
            <button
              key={item.id}
              role="menuitem"
              className={`menu-item${item.danger ? " danger" : ""}`}
              data-testid={`${testId}-item-${item.id}`}
              onClick={() => {
                setOpen(false);
                item.onSelect?.();
              }}
            >
              <span>{item.label}</span>
              {item.hint && <kbd>{item.hint}</kbd>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Right-click menu positioned at the pointer. */
export function ContextMenu({
  at,
  items,
  onClose,
}: {
  at: { x: number; y: number } | null;
  items: MenuItem[];
  onClose: () => void;
}) {
  useEffect(() => {
    if (!at) return;
    const close = () => onClose();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("blur", close);
    };
  }, [at, onClose]);
  if (!at) return null;
  return (
    <div
      className="menu context-menu"
      role="menu"
      data-testid="context-menu"
      style={{ left: at.x, top: at.y }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {items.map((item) => (
        <button
          key={item.id}
          role="menuitem"
          className={`menu-item${item.danger ? " danger" : ""}`}
          onClick={() => {
            onClose();
            item.onSelect?.();
          }}
        >
          <span>{item.label}</span>
          {item.hint && <kbd>{item.hint}</kbd>}
        </button>
      ))}
    </div>
  );
}
