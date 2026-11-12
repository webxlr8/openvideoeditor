// Failures nobody waits for still reach the person: work started with `fire` / `runCommand` reports a rejected
// promise or an error result to the app's toast (or a caller's own `onError`), and logs it.

import type { AppError } from "./bindings";
import type { Result } from "./bridge";

export type ErrorSink = (e: AppError) => void;

let sink: ErrorSink | null = null;

/** The app registers its toast here; until it does, failures are only logged. Returns an unregister function. */
export function setErrorSink(fn: ErrorSink): () => void {
  sink = fn;
  return () => {
    if (sink === fn) sink = null;
  };
}

function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { code?: unknown }).code === "string" &&
    typeof (e as { message?: unknown }).message === "string"
  );
}

/** Any thrown value (a rejected command, a broken callback) as a plain-language error. */
export function asAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  const detail =
    e instanceof Error ? e.message : typeof e === "string" ? e : "";
  return {
    code: "internal",
    message: detail
      ? `Something went wrong: ${detail}`
      : "Something went wrong. Try again.",
  };
}

/** Logs a failure and shows it (to `onError` when given, otherwise the app's toast). */
export function reportError(e: unknown, onError?: ErrorSink): void {
  const err = asAppError(e);
  console.error("[ove]", err.message, e);
  (onError ?? sink)?.(err);
}

/** Logs a failure without interrupting the person (background bookkeeping like `reportUiState`). */
export function logError(e: unknown): void {
  console.error("[ove]", asAppError(e).message, e);
}

/** Starts async work nobody awaits; a rejection is reported instead of vanishing as an unhandled rejection. */
export function fire(p: Promise<unknown>, onError?: ErrorSink): void {
  p.catch((e: unknown) => reportError(e, onError));
}

/**
 * Runs a command nobody awaits. `onOk` gets the data; an error result or a rejection is reported to `onError`
 * (default: the app's toast).
 */
export function runCommand<T>(
  p: Promise<Result<T>>,
  onOk?: (data: T) => void,
  onError?: ErrorSink,
): void {
  p.then((r) => {
    if (r.status === "ok") onOk?.(r.data);
    else if (onError) onError(r.error);
    else reportError(r.error);
  }).catch((e: unknown) => reportError(e, onError));
}

/** Detaches an event listener registered with `events.*.listen` (a cleanup function for effects). */
export function unlisten(p: Promise<() => void>): void {
  p.then((f) => f()).catch(logError);
}
