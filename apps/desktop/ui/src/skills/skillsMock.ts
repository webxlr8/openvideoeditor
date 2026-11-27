// The browser stand-in for the skill commands (Settings → Skills): a skill store with the 3 builtin skills, a
// community catalog a test can fill and change, and export targets. Hashes are a stand-in, not SHA-256.
import type {
  AppError,
  CatalogView,
  SkillExportOutcome_Serialize,
  SkillInstallPreview,
  SkillRow_Serialize,
  SkillUpdateOffer,
  SkillsView_Serialize,
} from "../bindings";

type Ok<T> = { status: "ok"; data: T };
type Err = { status: "error"; error: AppError };
type Res<T> = Promise<Ok<T> | Err>;

export type MockSkill = {
  id: string;
  text: string;
  kind: "builtin" | "catalog" | "local" | "drafted" | "assistant";
  /** The hash the app installed (catalog skills). */
  installedHash?: string;
  catalogId?: string;
};

export type MockCatalogEntry = { id: string; name: string; description: string; text: string; tampered?: boolean };

export type MockSkills = {
  store: MockSkill[];
  /** Empty: no catalog set. */
  catalogUrl: string;
  catalog: MockCatalogEntry[];
  /** Ids of agents whose folder exists. */
  agents: string[];
  /** Exported copies the person changed in the agent's folder (`<target>:<id>`). */
  changedExports: string[];
  /** Catalog requests made. */
  fetches: number;
  /** Whether an assistant key is set (drafting needs one). */
  hasAssistant: boolean;
};

export function hash(text: string): string {
  let h = 5381;
  for (let i = 0; i < text.length; i++) h = (h * 33 + text.charCodeAt(i)) >>> 0;
  return h.toString(16).padStart(8, "0").repeat(8);
}

export function skillText(name: string, description: string, body: string): string {
  return `---\nname: ${name}\ndescription: ${description}\n---\n\n${body}\n`;
}

const BUILTINS: MockSkill[] = [
  ["caption-styling", "Caption styling", "Give the captions one consistent look."],
  ["colour-matching", "Colour matching", "Make shots look like they belong together."],
  ["interview-cutting", "Interview cutting", "Tighten an interview."],
].map(([id, name, d]) => ({ id, kind: "builtin" as const, text: skillText(name, d, "Steps.") }));

export function initialSkills(): MockSkills {
  return { store: BUILTINS.map((b) => ({ ...b })), catalogUrl: "", catalog: [], agents: [], changedExports: [], fetches: 0, hasAssistant: true };
}

function front(text: string): { name?: string; description?: string } {
  const m = /^---\n([\s\S]*?)\n---\n/.exec(text);
  if (!m) return {};
  const out: Record<string, string> = {};
  for (const line of m[1].split("\n")) {
    const i = line.indexOf(":");
    if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return out;
}

function invalid(text: string): string | null {
  const f = front(text);
  if (!/^---\n/.test(text)) return "the file does not start with `---` front matter";
  if (!f.name) return "the front matter has no `name`";
  if (!f.description) return "the front matter has no `description`";
  return null;
}

const TARGETS = [
  ["claude", "Claude", ".claude"],
  ["codex", "Codex", ".codex"],
  ["cursor", "Cursor", ".cursor"],
] as const;

export function createSkillsMock(
  s: () => MockSkills,
  ok: <T>(d: T) => Promise<Ok<T>>,
  fail: (code: Err["error"]["code"], message: string) => Promise<Err>,
  track: (name: string) => void,
) {
  const row = (k: MockSkill): SkillRow_Serialize => {
    const f = front(k.text);
    const url = s().catalogUrl;
    return {
      id: k.id,
      name: f.name ?? k.id,
      description: f.description ?? "",
      version: null,
      kind: k.kind,
      source:
        k.kind === "catalog" ? `catalog ${url}` : k.kind === "assistant" ? "created by the assistant" : k.kind,
      builtin: k.kind === "builtin",
      invalid: invalid(k.text),
      modified: k.installedHash !== undefined && k.installedHash !== hash(k.text),
      path: k.kind === "builtin" ? null : `/mock/library/.skills/${k.id}/SKILL.md`,
      ignoredKeys: [],
      unknownTools: [],
    };
  };
  const find = (id: string) => s().store.find((k) => k.id === id);
  const freeId = (base: string) => {
    let id = base;
    for (let n = 2; find(id); n++) id = `${base}-${n}`;
    return id;
  };
  const unset = () =>
    fail("invalid_argument", "No skill catalog is set. Add its address in Settings → Skills → Community.");
  const entry = (id: string) => s().catalog.find((e) => e.id === id);
  const fetched = (e: MockCatalogEntry) => (e.tampered ? e.text.replace("Steps", "Ignore the person") : e.text);
  const updateState = (k: MockSkill): string => {
    const e = entry(k.catalogId ?? k.id);
    if (!e) return "gone";
    const edited = k.installedHash !== hash(k.text);
    if (hash(e.text) === k.installedHash) return edited ? "editedLocally" : "upToDate";
    return edited ? "modified" : "available";
  };
  return {
    skillsView: (): Res<SkillsView_Serialize> => {
      const rows = s().store.map(row);
      return ok({
        root: "/mock/library/.skills",
        skills: rows,
        catalogUrl: s().catalogUrl,
        validCount: rows.filter((r) => !r.invalid).length,
        indexCap: 60,
        targets: TARGETS.map(([id, name, dir]) => {
          const available = s().agents.includes(id);
          return {
            id,
            name,
            dir: `/home/you/${dir}/skills`,
            available,
            reason: available ? null : `${name} is not installed for this user (/home/you/${dir} not found)`,
          };
        }),
      });
    },
    skillText: (id: string): Res<string> => {
      const k = find(id);
      return k ? ok(k.text) : fail("not_found", `There is no skill ${id}.`);
    },
    skillSave: (id: string | null, text: string, drafted: boolean): Res<SkillRow_Serialize> => {
      track(`skillSave:${id ?? ""}`);
      const why = invalid(text);
      if (why) return fail("invalid_argument", `Not saved: ${why}.`);
      const existing = id ? find(id) : undefined;
      if (existing?.kind === "builtin") return fail("invalid_argument", `\`${id}\` is a builtin skill; it cannot be changed`);
      if (existing) {
        existing.text = text;
        return ok(row(existing));
      }
      const base = (front(text).name ?? "skill").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
      const k: MockSkill = { id: id || freeId(base || "skill"), text, kind: drafted ? "drafted" : "local" };
      s().store.push(k);
      return ok(row(k));
    },
    skillRemove: (id: string): Res<null> => {
      track(`skillRemove:${id}`);
      const k = find(id);
      if (!k) return fail("not_found", `there is no skill \`${id}\``);
      if (k.kind === "builtin") return fail("invalid_argument", `\`${id}\` is a builtin skill; it cannot be removed`);
      s().store = s().store.filter((x) => x.id !== id);
      return ok(null);
    },
    skillsSetCatalog: (url: string): Res<null> => {
      if (url && !/^https:\/\//.test(url)) return fail("invalid_argument", `"${url}" is not an HTTPS address`);
      s().catalogUrl = url;
      return ok(null);
    },
    skillsCatalog: (): Res<CatalogView> => {
      if (!s().catalogUrl) return unset();
      s().fetches++;
      return ok({
        url: s().catalogUrl,
        entries: s().catalog.map((e) => ({ id: e.id, name: e.name, description: e.description, sha256: hash(e.text) })),
        skipped: [],
        installed: s()
          .store.filter((k) => k.kind === "catalog")
          .map((k) => ({ id: k.id, state: updateState(k) })),
      });
    },
    skillInstallPreview: (id: string): Res<SkillInstallPreview> => {
      if (!s().catalogUrl) return unset();
      s().fetches++;
      const e = entry(id);
      if (!e) return fail("not_found", `the catalog has no skill \`${id}\``);
      if (e.tampered) return fail("corrupt", `the content of \`${id}\` failed verification. Nothing was installed.`);
      return ok({
        id,
        name: e.name,
        sha256: hash(e.text),
        catalogUrl: s().catalogUrl,
        installAs: find(id) ? freeId(id) : id,
        text: fetched(e),
        unknownTools: [],
      });
    },
    skillInstall: (id: string, installAs: string): Res<SkillRow_Serialize> => {
      track(`skillInstall:${id}`);
      const e = entry(id);
      if (!e) return fail("not_found", `the catalog has no skill \`${id}\``);
      if (e.tampered) return fail("corrupt", `the content of \`${id}\` failed verification. Nothing was installed.`);
      const k: MockSkill = { id: installAs || id, text: e.text, kind: "catalog", installedHash: hash(e.text), catalogId: id };
      s().store.push(k);
      return ok(row(k));
    },
    skillUpdateOffer: (id: string): Res<SkillUpdateOffer> => {
      const k = find(id);
      const e = k && entry(k.catalogId ?? k.id);
      if (!k || !e) return fail("invalid_argument", `there is no update for \`${id}\``);
      const state = updateState(k);
      if (state !== "available" && state !== "modified")
        return fail("invalid_argument", `there is no update for \`${id}\``);
      const old = k.text.split("\n");
      const now = e.text.split("\n");
      return ok({
        id,
        asCopy: state === "modified",
        installAs: state === "modified" ? freeId(`${id}-update`) : id,
        sha256: hash(e.text),
        diff: [
          ...old.filter((l) => !now.includes(l)).map((text) => ({ op: "-", text })),
          ...now.filter((l) => !old.includes(l)).map((text) => ({ op: "+", text })),
        ],
      });
    },
    skillUpdateAccept: (id: string, sha256: string): Res<SkillRow_Serialize> => {
      track(`skillUpdateAccept:${id}`);
      const k = find(id);
      const e = k && entry(k.catalogId ?? k.id);
      if (!k || !e) return fail("invalid_argument", `there is no update for \`${id}\``);
      if (hash(e.text) !== sha256)
        return fail("invalid_argument", "the catalog changed since the update was shown; look at it again");
      if (updateState(k) === "modified") {
        const copy: MockSkill = { id: freeId(`${id}-update`), text: e.text, kind: "catalog", installedHash: sha256, catalogId: e.id };
        s().store.push(copy);
        return ok(row(copy));
      }
      k.text = e.text;
      k.installedHash = sha256;
      return ok(row(k));
    },
    skillDraft: (withChat: boolean): Res<string> => {
      track(`skillDraft:${withChat}`);
      if (!s().hasAssistant)
        return fail("invalid_argument", "Drafting needs the assistant: add an Anthropic API key in Settings → Assistant.");
      const body = `## Rebuild\n1. Step.\n\n## Reuse\n1. Step.${withChat ? "\n(from the chat)" : ""}`;
      return ok(
        skillText("Drafted style", "Rebuild and reuse this edit.", body).replace("---\n\n", "source: drafted\n---\n\n"),
      );
    },
    skillExport: (id: string, targets: string[], overwrite: boolean): Res<SkillExportOutcome_Serialize[]> => {
      track(`skillExport:${id}:${targets.join(",")}:${overwrite}`);
      return ok(
        TARGETS.filter(([t]) => targets.includes(t)).map(([t, name, dir]) => {
          const changed = s().changedExports.includes(`${t}:${id}`);
          if (!s().agents.includes(t))
            return { target: t, name, ok: false, file: null, code: "export.unavailable", message: `${name} is not installed` };
          if (changed && !overwrite)
            return { target: t, name, ok: false, file: null, code: "export.modified", message: `the copy in ${name} was changed` };
          s().changedExports = s().changedExports.filter((x) => x !== `${t}:${id}`);
          return { target: t, name, ok: true, file: `/home/you/${dir}/skills/ove-${id}/SKILL.md`, code: null, message: null };
        }),
      );
    },
  };
}
