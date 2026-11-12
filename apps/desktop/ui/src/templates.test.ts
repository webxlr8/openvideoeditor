import { describe, expect, it } from "vitest";
import { PROJECT_TEMPLATES } from "./templates";

// The assistant edits the timeline; it can't transcribe, watch or listen to media, remove silences or filler words,
// track a speaker, or generate media. Briefs must not ask it to.
const PROMISES_WE_CANNOT_KEEP = [
  /\btranscribe\b/i,
  /\bsilences?\b/i,
  /\bfiller\b/i,
  /\bpauses\b/i,
  /frame the speaker/i,
  /reframe them/i,
  /on the beat/i,
  /generate the voice/i,
  /generated images/i,
];

describe("project templates", () => {
  it("only brief the assistant on what the app can do", () => {
    for (const t of PROJECT_TEMPLATES)
      for (const re of PROMISES_WE_CANNOT_KEEP)
        expect(`${t.id}: ${t.brief}`).not.toMatch(re);
  });
});
