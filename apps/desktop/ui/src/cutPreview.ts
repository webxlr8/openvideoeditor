// The ranges a previewed transcript cut would remove (Transcript panel), drawn over the clips on the timeline until
// the person applies or cancels the cut.
import { useSyncExternalStore } from "react";

export type CutRange = { start: number; end: number };

let ranges: CutRange[] = [];
const listeners = new Set<() => void>();

export function setCutPreview(next: CutRange[]) {
  ranges = next;
  listeners.forEach((l) => l());
}

export function useCutPreview(): CutRange[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => ranges,
  );
}
