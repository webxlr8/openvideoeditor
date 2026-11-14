// Onboarding for the app window: reads the state from the native side, decides whether the welcome shows, and
// ticks checklist steps as the open project (or a finished export) shows them done.

import { useCallback, useEffect, useRef, useState } from "react";
import type { ProjectView_Serialize } from "../bindings";
import { commands, events } from "../bridge";
import { logError, unlisten } from "../notify";
import { stepsShownBy } from "./checklistSteps";
import {
  type OnboardingChange,
  type OnboardingState,
  SETTLED,
  applyChange,
} from "./state";

export interface Onboarding {
  /** null until the native side has answered. */
  state: OnboardingState | null;
  /** The welcome covers the window: a first run, or Help → Show the welcome again. */
  welcomeOpen: boolean;
  showWelcome: () => void;
  /** Takes the welcome out of the way for this run without finishing it (it returns at the next launch). */
  standAside: () => void;
  /** The welcome's last screen was left with this assistant chosen (null: edit by hand). */
  finishWelcome: (assistant: string | null) => void;
  skipWelcome: () => void;
  setChecklistClosed: (closed: boolean) => void;
}

export function useOnboarding(project: ProjectView_Serialize | null): Onboarding {
  const [state, setState] = useState<OnboardingState | null>(null);
  const [again, setAgain] = useState(false);
  const [aside, setAside] = useState(false);

  useEffect(() => {
    let live = true;
    // Onboarding must never stand between the person and the app: if the state cannot be read, there is none.
    const settle = (e: unknown) => {
      logError(e);
      if (live) setState((s) => s ?? SETTLED);
    };
    commands
      .onboardingState()
      .then((r) => {
        if (r.status === "error") settle(r.error);
        else if (live) setState(r.data);
      })
      .catch(settle);
    return () => {
      live = false;
    };
  }, []);

  /** Shows the change at once; the native answer (the state as saved) replaces it. */
  const change = useCallback((c: OnboardingChange) => {
    setState((s) => (s ? applyChange(s, c) : s));
    commands
      .onboardingUpdate(c)
      .then((r) => {
        if (r.status === "ok") setState(r.data);
        else logError(r.error);
      })
      .catch(logError);
  }, []);

  // Steps the open project shows as done: each edit is compared with the project as it was before it.
  const before = useRef<ProjectView_Serialize | null>(null);
  const done = state?.stepsDone;
  useEffect(() => {
    const steps = stepsShownBy(before.current, project);
    before.current = project;
    const fresh = steps.filter((s) => !done?.includes(s));
    if (done && fresh.length > 0) change({ kind: "completeSteps", steps: fresh });
  }, [project, done, change]);

  // Export: ticked when an export finishes with a file.
  const exported = done?.includes("export") ?? true;
  useEffect(() => {
    if (exported) return;
    const l = events.exportProgress.listen((e) => {
      if (e.payload.done && !e.payload.error && e.payload.path)
        change({ kind: "completeSteps", steps: ["export"] });
    });
    return () => unlisten(l);
  }, [exported, change]);

  return {
    state,
    welcomeOpen: again || (!aside && state?.welcome === "pending"),
    showWelcome: useCallback(() => setAgain(true), []),
    standAside: useCallback(() => {
      setAgain(false);
      setAside(true);
    }, []),
    finishWelcome: useCallback(
      (assistant) => {
        setAgain(false);
        change({ kind: "finishWelcome", assistant });
      },
      [change],
    ),
    skipWelcome: useCallback(() => {
      setAgain(false);
      change({ kind: "skipWelcome" });
    }, [change]),
    setChecklistClosed: useCallback(
      (closed) => change({ kind: "setChecklistClosed", closed }),
      [change],
    ),
  };
}
