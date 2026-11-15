// Onboarding state as the UI holds it. The native side owns it (onboarding.json in the app's configuration
// folder); `applyChange` mirrors the native rules so the window can answer at once and the browser mock behaves
// the same. The native answer always replaces what is computed here.

import type {
  ChecklistStep,
  OnboardingChange_Deserialize as OnboardingChange,
  OnboardingState_Serialize as OnboardingState,
} from "../bindings";

export type { ChecklistStep, OnboardingChange, OnboardingState };

const ORDER: ChecklistStep[] = ["import", "cut", "title", "captions", "export"];

/** A first run: the welcome is due and nothing is ticked. */
export const FIRST_RUN: OnboardingState = {
  version: 1,
  welcome: "pending",
  assistant: null,
  stepsDone: [],
  checklistClosed: false,
};

/** Onboarding over and out of the way: what the app falls back to if the state cannot be read. */
export const SETTLED: OnboardingState = {
  ...FIRST_RUN,
  welcome: "finished",
  checklistClosed: true,
};

const chosen = (a: string | null | undefined) => (a && a.trim() ? a : null);

export function applyChange(
  state: OnboardingState,
  change: OnboardingChange,
): OnboardingState {
  switch (change.kind) {
    case "finishWelcome":
      return { ...state, welcome: "finished", assistant: chosen(change.assistant) };
    case "skipWelcome":
      // Skipping the first run also closes the checklist; skipping a welcome shown again changes nothing.
      return state.welcome === "pending"
        ? { ...state, welcome: "skipped", checklistClosed: true }
        : state;
    case "setAssistant":
      return { ...state, assistant: chosen(change.assistant) };
    case "completeSteps":
      return {
        ...state,
        stepsDone: ORDER.filter(
          (s) => state.stepsDone.includes(s) || change.steps.includes(s),
        ),
      };
    case "setChecklistClosed":
      return { ...state, checklistClosed: change.closed };
  }
}
