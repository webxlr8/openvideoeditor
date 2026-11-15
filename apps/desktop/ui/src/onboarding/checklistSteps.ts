// The guided first edit: five steps, each ticked when the project (or a finished export) shows it was done. Nothing
// here listens to buttons: an edit made by hand, by the assistant or from the command line ticks the same step.

import type {
  ChecklistStep,
  ItemView_Serialize,
  ProjectView_Serialize,
} from "../bindings";

export const STEPS: { id: ChecklistStep; label: string; how: string }[] = [
  {
    id: "import",
    label: "Import",
    how: "Add a video or sound file in the Media panel.",
  },
  {
    id: "cut",
    label: "Cut",
    how: "Select a clip, put the playhead inside it and press Split.",
  },
  { id: "title", label: "Title", how: "Pick a title style in the Text panel." },
  {
    id: "captions",
    label: "Captions",
    how: "Make captions from the speech in the Captions panel.",
  },
  { id: "export", label: "Export", how: "Press Export to save your video." },
];

const count = (v: ProjectView_Serialize, kind: string) =>
  v.items.filter((i) => i.kind === kind).length;

/** A clip was split: a new clip of the same media starts where an older one now ends, earlier than it did. */
function wasSplit(prev: ProjectView_Serialize, next: ProjectView_Serialize) {
  const before = new Map<string, ItemView_Serialize>(
    prev.items.map((i) => [i.id, i]),
  );
  return next.items.some(
    (right) =>
      !!right.mediaId &&
      !before.has(right.id) &&
      next.items.some((left) => {
        const was = before.get(left.id);
        return (
          !!was &&
          left.mediaId === right.mediaId &&
          left.trackId === right.trackId &&
          (left.end ?? 0) < (was.end ?? 0) - 1e-6 &&
          Math.abs((left.end ?? 0) - (right.start ?? 0)) < 1e-3
        );
      }),
  );
}

/**
 * The steps a change to the open project shows as done: `prev` and `next` are the same project before and after
 * an edit. Opening a project ticks nothing (what a project already holds, the sample's cut and title for one, was
 * not the person's first edit).
 */
export function stepsShownBy(
  prev: ProjectView_Serialize | null,
  next: ProjectView_Serialize | null,
): ChecklistStep[] {
  if (!prev || !next || prev.id !== next.id || prev.rev === next.rev) return [];
  const steps: ChecklistStep[] = [];
  if (next.media.length > prev.media.length) steps.push("import");
  if (wasSplit(prev, next)) steps.push("cut");
  if (count(next, "text") > count(prev, "text")) steps.push("title");
  if (count(next, "caption") > count(prev, "caption")) steps.push("captions");
  return steps;
}
