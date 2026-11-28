// The prepared requests panels hand to the assistant (design D2). One table of our own short prompts, so it can be
// translated in one place. Each request lists what it mentions; the person edits the request before sending it.

import type { ItemView_Serialize, ProjectView_Serialize } from "../bindings";
import { MARKED_RANGE, type MentionDraft, trackMention } from "./mentions";

export type HandoffPanel = "captions" | "audio" | "media" | "effects" | "transcript";

/** What a panel knows when the person picks a request. */
export interface HandoffContext {
  project: ProjectView_Serialize | null | undefined;
  selection: ItemView_Serialize[];
}

export interface PreparedRequest {
  id: string;
  /** The menu entry. */
  label: string;
  /** The request placed in the chat input. */
  prompt: string;
  /** What it mentions, from the panel's context. */
  mentions: (c: HandoffContext) => MentionDraft[];
}

/** Every track of a kind, as mentions. */
const tracksOf = (kind: string) => (c: HandoffContext) =>
  (c.project?.tracks ?? []).filter((t) => t.kind === kind).map((t) => trackMention(t.id, t.name));

/** The selected items, as mentions. */
const selected = (c: HandoffContext): MentionDraft[] =>
  c.selection.map((i) => ({
    kind: "item" as const,
    id: i.id,
    label: (i.kind === "text" || i.kind === "caption" ? i.text : null) ?? i.name ?? i.id,
    media: i.kind,
  }));

export const PREPARED: Record<HandoffPanel, PreparedRequest[]> = {
  captions: [
    {
      id: "fillers",
      label: "Remove filler words",
      prompt: "Remove the filler words (um, uh, you know, like) from these captions. Keep the meaning and the timing.",
      mentions: tracksOf("caption"),
    },
    {
      id: "names",
      label: "Fix names and jargon",
      prompt:
        "Check these captions for misspelled names, brands and technical terms, and correct them. Ask me when a spelling is not clear.",
      mentions: tracksOf("caption"),
    },
    {
      id: "translate",
      label: "Translate the captions",
      prompt: "Translate these captions into Spanish. Keep the timing of every caption.",
      mentions: tracksOf("caption"),
    },
  ],
  audio: [
    {
      id: "score",
      label: "Score this range",
      prompt:
        "Add music under this range that fits its mood, at a level that keeps the dialogue clear. Tell me what you picked.",
      mentions: (c) => [MARKED_RANGE, ...selected(c)],
    },
    {
      id: "duck",
      label: "Duck music under dialogue",
      prompt: "Lower the music wherever someone speaks, so the voices stay clear, and bring it back up in the gaps.",
      mentions: (c) => [...tracksOf("audio")(c), ...selected(c)],
    },
  ],
  media: [
    {
      id: "organise",
      label: "Organise the media",
      prompt:
        "Look through this project's media: suggest clear names for the files, and tell me which ones are not used on the timeline yet.",
      mentions: () => [],
    },
  ],
  effects: [
    {
      id: "match",
      label: "Match this shot to a reference",
      prompt:
        "Match the colour and brightness of the first clip I mention to the second one, the reference. Change only the first clip.",
      mentions: selected,
    },
  ],
  transcript: [
    {
      id: "tighten",
      label: "Tighten the edit",
      prompt:
        "Tighten this part: remove filler words and long pauses, but keep every sentence that carries meaning. Show me what you removed.",
      mentions: (c) => [MARKED_RANGE, ...selected(c)],
    },
  ],
};
