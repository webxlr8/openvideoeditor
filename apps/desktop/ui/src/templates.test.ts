import { describe, expect, it } from "vitest";
import {
  PROJECT_TEMPLATES,
  missingFields,
  templateAspect,
  templateDefaults,
  templateFields,
  templateMessage,
  templateProjectName,
} from "./templates";

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

  it("give every template a required subject and typed fields with unique ids", () => {
    for (const t of PROJECT_TEMPLATES) {
      const fields = templateFields(t);
      const ids = fields.map((f) => f.id);
      expect(new Set(ids).size, t.id).toBe(ids.length);
      expect(fields[0], t.id).toMatchObject({ id: "subject", required: true });
      for (const id of ["length", "aspect", "media", "details"])
        expect(ids, t.id).toContain(id);
      expect(templateDefaults(t), t.id).toMatchObject({ length: t.length, aspect: t.aspect });
    }
  });

  it("validate required fields before the assistant starts", () => {
    const t = PROJECT_TEMPLATES.find((x) => x.id === "product-launch")!;
    const values = templateDefaults(t);
    expect(missingFields(t, values)).toEqual([
      "Product name",
      "Three benefits",
      "Launch date or link",
    ]);
    values.subject = "Tidewater 2";
    values.benefits = "  ";
    expect(missingFields(t, values)).toEqual(["Three benefits", "Launch date or link"]);
    values.benefits = "Faster\nLighter\nCheaper";
    values.link = "tidewater.app";
    expect(missingFields(t, values)).toEqual([]);
  });

  it("compose the fields into the first message, the name and the frame", () => {
    const t = PROJECT_TEMPLATES.find((x) => x.id === "product-launch")!;
    const values = {
      ...templateDefaults(t),
      subject: "Tidewater 2",
      benefits: "Faster\nLighter",
      link: "tidewater.app",
      audience: "",
      aspect: "9:16",
    };
    const msg = templateMessage(t, values);
    expect(msg.startsWith(t.brief)).toBe(true);
    expect(msg).toContain("\n\nDetails:\n- Product name: Tidewater 2\n- Three benefits: Faster\n  Lighter\n");
    expect(msg).toContain("- Length: 30–45 s");
    expect(msg).toContain("- Frame: 9:16");
    expect(msg).not.toContain("Audience");
    expect(templateProjectName(t, values)).toBe("Product launch – Tidewater 2");
    expect(templateAspect(t, values)).toBe("9:16");
    expect(templateAspect(t, { aspect: "2:1" })).toBe("16:9");
    expect(templateMessage(t, {})).toBe(t.brief);
  });
});
