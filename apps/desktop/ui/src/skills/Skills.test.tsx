import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { initialSkills, skillText } from "./skillsMock";

const mock = () => window.__oveMock!;

async function openSkills() {
  render(<App />);
  fireEvent.click(screen.getByTestId("nav-settings"));
  fireEvent.click(await screen.findByTestId("set-skills"));
  await screen.findByTestId("skills-section");
}

describe("settings: skills", () => {
  beforeEach(() => {
    mock().skills = initialSkills();
  });

  it("lists the builtin skills and opens them read-only", async () => {
    mock().skills.store.push({ id: "broken", kind: "local", text: "---\nname: Broken\n---\nbody\n" });
    await openSkills();
    for (const id of ["caption-styling", "colour-matching", "interview-cutting"]) {
      expect(await screen.findByTestId(`skill-${id}`)).toBeTruthy();
      expect(screen.getByTestId(`skill-source-${id}`).textContent).toBe("builtin");
      expect(screen.queryByTestId(`skill-remove-${id}`)).toBeNull();
    }
    fireEvent.click(screen.getByTestId("skill-open-caption-styling"));
    const text = await screen.findByTestId<HTMLTextAreaElement>("skill-editor-text");
    expect(text.readOnly).toBe(true);
    expect(screen.queryByTestId("skill-editor-save")).toBeNull();
    // An invalid skill of the person's is listed with the reason.
    fireEvent.click(screen.getByTestId("skills-tab-local"));
    expect((await screen.findByTestId("skill-invalid-broken")).textContent).toContain("no `description`");
  });

  it("creates and edits a skill of the person's own", async () => {
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-local"));
    fireEvent.click(screen.getByTestId("skill-new"));
    const text = await screen.findByTestId<HTMLTextAreaElement>("skill-editor-text");
    fireEvent.change(text, { target: { value: skillText("Tidy cuts", "Close small gaps.", "Steps.") } });
    fireEvent.click(screen.getByTestId("skill-editor-save"));
    expect((await screen.findByTestId("skills-message")).textContent).toBe("Saved tidy-cuts.");
    expect(screen.getByTestId("skill-source-tidy-cuts").textContent).toBe("local");

    fireEvent.click(screen.getByTestId("skill-open-tidy-cuts"));
    const again = await screen.findByTestId<HTMLTextAreaElement>("skill-editor-text");
    fireEvent.change(again, { target: { value: skillText("Tidy cuts", "Close every gap.", "Steps.") } });
    fireEvent.click(screen.getByTestId("skill-editor-save"));
    await waitFor(() => expect(screen.getByTestId("skill-tidy-cuts").textContent).toContain("Close every gap."));
    expect(mock().calls).toContain("skillSave:tidy-cuts");
  });

  it("explains how to add a catalog and fetches nothing until one is set", async () => {
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-community"));
    expect((await screen.findByTestId("skills-community-unset")).textContent).toContain("No community catalog is set");
    expect(mock().skills.fetches).toBe(0);
  });

  it("installs a catalog skill only after showing it", async () => {
    mock().skills.catalogUrl = "https://skills.example/catalog.json";
    mock().skills.catalog = [
      { id: "pacing", name: "Pacing", description: "Keep a steady pace.", text: skillText("Pacing", "Keep a steady pace.", "Steps.") },
    ];
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-community"));
    fireEvent.click(await screen.findByTestId("catalog-install-pacing"));
    expect((await screen.findByTestId("skill-install-text")).textContent).toContain("Keep a steady pace.");
    expect(mock().calls).not.toContain("skillInstall:pacing");
    fireEvent.click(screen.getByTestId("skill-install-confirm"));
    expect((await screen.findByTestId("skills-community-message")).textContent).toBe("Installed pacing.");
    fireEvent.click(screen.getByTestId("skills-tab-installed"));
    expect((await screen.findByTestId("skill-source-pacing")).textContent).toBe(
      "catalog https://skills.example/catalog.json",
    );
  });

  it("installs only the text it showed", async () => {
    const s = mock().skills;
    s.catalogUrl = "https://skills.example/catalog.json";
    s.catalog = [{ id: "pacing", name: "Pacing", description: "d", text: skillText("Pacing", "d", "Steps.") }];
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-community"));
    fireEvent.click(await screen.findByTestId("catalog-install-pacing"));
    await screen.findByTestId("skill-install-text");
    // The catalog serves other text after the preview.
    s.catalog = [{ id: "pacing", name: "Pacing", description: "d", text: skillText("Pacing", "d", "Other steps.") }];
    fireEvent.click(screen.getByTestId("skill-install-confirm"));
    expect((await screen.findByTestId("skills-community-message")).textContent).toContain("changed since it was shown");
    expect(s.store.some((k) => k.id === "pacing")).toBe(false);
  });

  it("refuses tampered catalog content", async () => {
    mock().skills.catalogUrl = "https://skills.example/catalog.json";
    mock().skills.catalog = [
      { id: "pacing", name: "Pacing", description: "d", text: skillText("Pacing", "d", "Steps."), tampered: true },
    ];
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-community"));
    fireEvent.click(await screen.findByTestId("catalog-install-pacing"));
    expect((await screen.findByTestId("skills-community-message")).textContent).toContain("failed verification");
    expect(screen.queryByTestId("skill-install-preview")).toBeNull();
    expect(mock().skills.store.some((k) => k.id === "pacing")).toBe(false);
  });

  it("shows an update's changes and installs it on accept", async () => {
    const v1 = skillText("Pacing", "Keep a steady pace.", "Cut on the beat.");
    const v2 = skillText("Pacing", "Keep a steady pace.", "Cut on the beat.\nHold wide shots longer.");
    const s = mock().skills;
    s.catalogUrl = "https://skills.example/catalog.json";
    s.catalog = [{ id: "pacing", name: "Pacing", description: "d", text: v2 }];
    const { hash } = await import("./skillsMock");
    s.store.push({ id: "pacing", kind: "catalog", text: v1, installedHash: hash(v1), catalogId: "pacing" });
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-community"));
    fireEvent.click(await screen.findByTestId("skill-update-review-pacing"));
    const diff = await screen.findByTestId("skill-update-diff");
    expect(diff.textContent).toContain("+Hold wide shots longer.");
    expect(s.store.find((k) => k.id === "pacing")!.text).toBe(v1);
    fireEvent.click(screen.getByTestId("skill-update-accept"));
    expect((await screen.findByTestId("skills-community-message")).textContent).toBe("Updated pacing.");
    expect(s.store.find((k) => k.id === "pacing")!.text).toBe(v2);
  });

  it("keeps an edited skill and offers its update as a copy", async () => {
    const v1 = skillText("Pacing", "Keep a steady pace.", "Cut on the beat.");
    const v2 = skillText("Pacing", "Keep a steady pace.", "Cut on the downbeat.");
    const s = mock().skills;
    s.catalogUrl = "https://skills.example/catalog.json";
    s.catalog = [{ id: "pacing", name: "Pacing", description: "d", text: v2 }];
    const { hash } = await import("./skillsMock");
    const mine = v1.replace("beat", "bar");
    s.store.push({ id: "pacing", kind: "catalog", text: mine, installedHash: hash(v1), catalogId: "pacing" });
    await openSkills();
    expect((await screen.findByTestId("skill-modified-pacing")).textContent).toBe("modified");
    fireEvent.click(screen.getByTestId("skills-tab-community"));
    expect((await screen.findByTestId("skill-update-pacing")).textContent).toContain("comes as a copy");
    fireEvent.click(screen.getByTestId("skill-update-review-pacing"));
    fireEvent.click(await screen.findByTestId("skill-update-accept"));
    expect((await screen.findByTestId("skills-community-message")).textContent).toBe(
      "Installed the update as pacing-update.",
    );
    expect(s.store.find((k) => k.id === "pacing")!.text).toBe(mine);
  });

  it("exports to installed agents, names unavailable ones, and asks before replacing a changed copy", async () => {
    mock().skills.agents = ["claude"];
    mock().skills.changedExports = ["claude:interview-cutting"];
    await openSkills();
    fireEvent.click(await screen.findByTestId("skill-export-interview-cutting"));
    expect(screen.getByTestId("skill-export-reason-cursor").textContent).toContain("Cursor is not installed");
    expect(screen.getByTestId<HTMLButtonElement>("skill-export-to-cursor").disabled).toBe(true);
    fireEvent.click(screen.getByTestId("skill-export-to-claude"));
    const result = await screen.findByTestId("skill-export-result-claude");
    expect(result.textContent).toContain("You changed the copy in Claude");
    expect(mock().calls).toContain("skillExport:interview-cutting:claude:false");
    fireEvent.click(screen.getByTestId("skill-export-replace-claude"));
    await waitFor(() =>
      expect(screen.getByTestId("skill-export-result-claude").textContent).toContain(
        "/home/you/.claude/skills/ove-interview-cutting/SKILL.md",
      ),
    );
    expect(mock().calls).toContain("skillExport:interview-cutting:claude:true");
  });

  it("drafts a skill from the open project into the editor and saves it only on Save", async () => {
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-local"));
    fireEvent.click(screen.getByTestId("skill-draft-chat"));
    fireEvent.click(screen.getByTestId("skill-draft"));
    const text = await screen.findByTestId<HTMLTextAreaElement>("skill-editor-text");
    expect(text.value).toContain("source: drafted");
    expect(text.value).toContain("(from the chat)");
    expect(mock().calls).toContain("skillDraft:true");
    expect(mock().skills.store.some((k) => k.kind === "drafted")).toBe(false);
    fireEvent.click(screen.getByTestId("skill-editor-save"));
    expect((await screen.findByTestId("skill-source-drafted-style")).textContent).toBe("drafted");
  });

  it("says drafting needs the assistant when none is set", async () => {
    mock().skills.hasAssistant = false;
    await openSkills();
    fireEvent.click(screen.getByTestId("skills-tab-local"));
    fireEvent.click(screen.getByTestId("skill-draft"));
    expect((await screen.findByTestId("skills-message")).textContent).toContain("Drafting needs the assistant");
    expect(screen.queryByTestId("skill-editor")).toBeNull();
  });

  it("warns above 60 skills", async () => {
    for (let i = 0; i < 60; i++)
      mock().skills.store.push({ id: `s${i}`, kind: "local", text: skillText(`S${i}`, "d", "b") });
    await openSkills();
    expect((await screen.findByTestId("skills-cap-warning")).textContent).toContain("63 skills");
  });
});
