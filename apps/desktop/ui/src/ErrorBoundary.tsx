// Error boundaries: a render error in one region (timeline, viewer, a panel, the assistant, …) replaces only that
// region with a small card; the rest of the window keeps working. Errors are logged by the root's
// onCaughtError / onUncaughtError handlers (main.tsx).

import { Component, Fragment, type ReactNode } from "react";

interface Props {
  /** What the person calls this part of the window, e.g. "The timeline". */
  region: string;
  /** Class of the region's own root, so the card takes the region's place in the layout. */
  className?: string;
  /** When any of these change, a failed region tries again (e.g. another panel or item was picked). */
  resetKeys?: unknown[];
  /** Shown as a second button when the region can be closed (dialogs). */
  onClose?: () => void;
  reloadLabel?: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
  /** Bumped by "Reload" so the region mounts afresh. */
  generation: number;
}

function changed(a: unknown[] = [], b: unknown[] = []) {
  return a.length !== b.length || a.some((v, i) => !Object.is(v, b[i]));
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, generation: 0 };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return {
      error: error instanceof Error ? error : new Error(String(error)),
    };
  }

  componentDidUpdate(prev: Props) {
    if (this.state.error && changed(prev.resetKeys, this.props.resetKeys))
      this.reload();
  }

  reload = () =>
    this.setState((s) => ({ error: null, generation: s.generation + 1 }));

  render() {
    const { error, generation } = this.state;
    if (!error)
      return <Fragment key={generation}>{this.props.children}</Fragment>;
    const { region, className, onClose, reloadLabel } = this.props;
    return (
      <div
        className={`region-error${className ? ` ${className}` : ""}`}
        role="alert"
        data-testid="region-error"
        data-region={region}
      >
        <div className="region-error-card">
          <strong>{region} ran into a problem.</strong>
          <p className="muted small">
            Your project is saved. The rest of the window still works.
          </p>
          <p className="region-error-detail small" title={error.message}>
            {error.message}
          </p>
          <div className="region-error-actions">
            {onClose && (
              <button className="btn" onClick={onClose}>
                Close
              </button>
            )}
            <button
              className="btn primary"
              onClick={this.reload}
              data-testid="region-reload"
            >
              {reloadLabel ?? "Reload this panel"}
            </button>
          </div>
        </div>
      </div>
    );
  }
}

/** Logs a render error with the component stack (the backend has no UI log command). */
export function logRenderError(
  kind: "caught" | "uncaught",
  error: unknown,
  info: { componentStack?: string | null },
) {
  console.error(
    `[ove] ${kind} render error:`,
    error,
    info.componentStack ?? "",
  );
}
