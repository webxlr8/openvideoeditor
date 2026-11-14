// JS licence gate (openspec/config.yaml hard rules). Runtime (bundled) packages must use one of
// ALLOWED. Build-time-only packages may additionally use DEV_ONLY licences: they never ship in
// the app.
//
// DEV_ONLY rationale:
// - CC-BY-4.0: caniuse-lite (browser support data read by the bundler at build time).
// - BlueOak-1.0.0: permissive licence used by small build-tool helpers (lru-cache, minimatch).
// - Python-2.0: argparse, if a tool pulls it in.
import { execFileSync } from "node:child_process";

const ALLOWED = new Set([
  "MIT",
  "MIT-0",
  "ISC",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
  "Zlib",
  "MPL-2.0",
  "CC0-1.0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
]);
const DEV_ONLY = new Set(["CC-BY-4.0", "BlueOak-1.0.0", "Python-2.0"]);

/** An SPDX expression passes if every OR-alternative set contains an allowed choice. */
function permitted(expr, allowed) {
  const cleaned = expr.replace(/[()]/g, "").trim();
  return cleaned.split(/\s+OR\s+/).some((alt) => alt.split(/\s+AND\s+/).every((id) => allowed.has(id.trim())));
}

function list(prod) {
  const args = ["licenses", "list", "--json"];
  if (prod) args.push("--prod");
  // pnpm is pnpm.cmd on Windows, which only a shell can start (the arguments are fixed literals).
  const out = execFileSync("pnpm", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    shell: process.platform === "win32",
  });
  return JSON.parse(out || "{}");
}

let failures = 0;
const prod = list(true);
for (const [licence, pkgs] of Object.entries(prod)) {
  if (!permitted(licence, ALLOWED)) {
    for (const p of pkgs) console.error(`runtime dependency ${p.name}: licence ${licence} is not allowed`);
    failures += pkgs.length;
  }
}
const all = list(false);
const devAllowed = new Set([...ALLOWED, ...DEV_ONLY]);
let count = 0;
for (const [licence, pkgs] of Object.entries(all)) {
  count += pkgs.length;
  if (!permitted(licence, devAllowed)) {
    for (const p of pkgs) console.error(`build dependency ${p.name}: licence ${licence} is not allowed`);
    failures += pkgs.length;
  }
}
if (failures > 0) {
  console.error(`${failures} package(s) fail the licence policy`);
  process.exit(1);
}
console.log(`licences ok: ${count} packages (${Object.values(prod).flat().length} runtime)`);
