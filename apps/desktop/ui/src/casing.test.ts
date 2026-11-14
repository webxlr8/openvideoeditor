// macOS and Windows treat `Checklist.tsx` and `checklist.ts` as the same file name, so an import of one can resolve
// to the other and the build fails there while it passes on Linux. This keeps module names in a folder distinct
// without regard to letter case or extension.
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function modules(dir: string): string[] {
  const out: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...modules(full));
    else if (/\.(ts|tsx)$/.test(entry.name) && !/\.(test|d)\.tsx?$/.test(entry.name)) out.push(full);
  }
  return out;
}

describe("source file names", () => {
  it("do not differ only by letter case or extension within a folder", () => {
    const seen = new Map<string, string>();
    const clashes: string[] = [];
    for (const file of modules(path.resolve(__dirname))) {
      const key = path.join(path.dirname(file), path.basename(file).replace(/\.tsx?$/, "")).toLowerCase();
      const other = seen.get(key);
      if (other) clashes.push(`${path.relative(__dirname, other)} and ${path.relative(__dirname, file)}`);
      else seen.set(key, file);
    }
    expect(clashes).toEqual([]);
  });
});
