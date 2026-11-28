// Panel hand-off (change add-agent-chat-context-and-activity, design D2): a panel that knows the context opens the
// chat with a prepared request and its mentions. The request goes into the input for the person to edit; it is never
// sent automatically.

import type { MentionDraft } from "./mentions";

export interface AssistantRequest {
  /** The request text, placed in the chat input. */
  prompt: string;
  /** Mentions placed as chips next to it. */
  mentions?: MentionDraft[];
  /** Start a new conversation for it. */
  newChat?: boolean;
}

type Listener = (r: AssistantRequest) => void;

let pending: AssistantRequest | null = null;
const listeners = new Set<Listener>();

/** Opens the chat with a prepared request (the editor opens the chat; the chat fills its input). */
export function askAssistant(request: AssistantRequest) {
  pending = request;
  for (const l of [...listeners]) l(request);
}

/** The request waiting for a chat to take it, if any (a chat that mounts later takes it then). */
export function takeAssistantRequest(): AssistantRequest | null {
  const r = pending;
  pending = null;
  return r;
}

/** Calls `listener` on every request; returns the unsubscribe function. */
export function onAssistantRequest(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
