// The list that opens over the chat input: the @ mention popover (with its media tabs) and the `/` menu. Keyboard
// handling stays in the input (arrows move, Enter or Tab picks, Escape closes); this only draws the rows.

import { Icon, type IconName } from "../icons";

export interface MenuRow {
  key: string;
  label: string;
  hint?: string;
  icon?: IconName;
}

export function ComposerMenu({
  testId,
  title,
  rows,
  active,
  onPick,
  onHover,
  tabs,
  tab,
  onTab,
}: {
  testId: string;
  title: string;
  rows: MenuRow[];
  active: number;
  onPick: (index: number) => void;
  onHover?: (index: number) => void;
  tabs?: { id: string; label: string }[];
  tab?: string;
  onTab?: (id: string) => void;
}) {
  return (
    <div className="composer-menu" role="listbox" aria-label={title} data-testid={testId}>
      {tabs && (
        <div className="composer-menu-tabs" role="tablist">
          {tabs.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={t.id === tab}
              className={`composer-menu-tab${t.id === tab ? " on" : ""}`}
              // The input keeps the focus.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onTab?.(t.id)}
              data-testid={`${testId}-tab-${t.id}`}
            >
              {t.label}
            </button>
          ))}
        </div>
      )}
      {rows.length === 0 ? (
        <div className="composer-menu-empty muted small" data-testid={`${testId}-empty`}>
          No matches
        </div>
      ) : (
        <ul>
          {rows.map((r, i) => (
            <li key={r.key}>
              <button
                role="option"
                aria-selected={i === active}
                className={`composer-menu-row${i === active ? " on" : ""}`}
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => onHover?.(i)}
                onClick={() => onPick(i)}
                data-testid={`${testId}-row`}
              >
                {r.icon && <Icon name={r.icon} size={13} />}
                <span className="grow">{r.label}</span>
                {r.hint && <span className="muted small">{r.hint}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
