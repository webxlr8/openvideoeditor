// End-to-end suite for the real desktop app (Tauri + React + Rust), driven through tauri-driver (WebDriver).
// Not the browser mock: every flow runs against the debug build of `ove-desktop`.
//
// Prerequisites
//   - the debug app:          cd apps/desktop && pnpm tauri build --debug --no-bundle
//   - the mock Messages API:  cargo build -p ove-agent --example mock_server
//   - tauri-driver:           cargo install tauri-driver --locked
//   - the native driver:      Linux: WebKitWebDriver (apt install webkit2gtk-driver);
//                             Windows: msedgedriver matching the installed WebView2 (msedgedriver-tool)
//   - test media:             python3 tests/media/media.py generate
//   - speech for captions:    espeak-ng (Linux) or the Windows speech synthesiser
// Usage
//   pnpm e2e:app                         (Linux needs a display: xvfb-run -a pnpm e2e:app)
//   pnpm e2e:app -- --only timeline,export
// Environment
//   OVE_E2E_APP            the app binary (default: $CARGO_TARGET_DIR or <repo>/target, debug/ove-desktop)
//   OVE_E2E_MOCK_SERVER    the mock_server example binary (default next to the app, debug/examples/mock_server)
//   OVE_E2E_TAURI_DRIVER   tauri-driver (default: on PATH, else ~/.cargo/bin)
//   OVE_E2E_NATIVE_DRIVER  WebKitWebDriver / msedgedriver path, passed to tauri-driver --native-driver
//   OVE_E2E_MEDIA          test media folder (default <repo>/tests/media/files)
//   OVE_E2E_OUT            artefacts folder (default apps/desktop/e2e-app/artefacts): screenshots, page sources and
//                          the app log of failed flows, plus the full app log, driver logs and summary.json
//   OVE_WHISPER_MODEL_DIR  speech models (cache it in CI; default <tmp>/ove-e2e-models)
// First run
//   The first flow walks the welcome a new person sees. Linux gives the app a fresh home, so it is always a first
//   run there; elsewhere the flow reopens the welcome from Help when the profile is not new. Every other launch
//   skips the welcome if it shows (lib/session.mjs).
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { FLOWS } from "./flows.mjs";
import { AppSession } from "./lib/session.mjs";
import {
  ALLOWED_ERRORS,
  WINDOWS,
  appRunning,
  badLogRecords,
  closeAppWindow,
  freePort,
  killApp,
  logRecords,
  sleep,
  startMockMessages,
  startTauriDriver,
  stop,
  which,
} from "./lib/support.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../..");
const exe = (n) => (WINDOWS ? `${n}.exe` : n);
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(repo, "target");
const APP = path.resolve(process.env.OVE_E2E_APP ?? path.join(targetDir, "debug", exe("ove-desktop")));
const APP_EXE = path.basename(APP);
const MOCK = path.resolve(process.env.OVE_E2E_MOCK_SERVER ?? path.join(targetDir, "debug", "examples", exe("mock_server")));
const TAURI_DRIVER =
  process.env.OVE_E2E_TAURI_DRIVER ?? which("tauri-driver") ?? path.join(os.homedir(), ".cargo", "bin", exe("tauri-driver"));
const MEDIA = path.resolve(process.env.OVE_E2E_MEDIA ?? path.join(repo, "tests", "media", "files"));
const OUT = path.resolve(process.env.OVE_E2E_OUT ?? path.join(here, "artefacts"));

const args = process.argv.slice(2);
const onlyArg = args.includes("--only") ? args[args.indexOf("--only") + 1] : null;
const only = onlyArg ? onlyArg.split(",").map((s) => s.trim().toLowerCase()) : null;
const keep = args.includes("--keep");

const log = (s) => console.log(s);

for (const [what, p] of [
  ["app", APP],
  ["mock Messages server", MOCK],
  ["test media", MEDIA],
]) {
  if (!fs.existsSync(p)) {
    console.error(`missing ${what}: ${p} (see the header of e2e-app/run.mjs)`);
    process.exit(2);
  }
}

fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });
const work = fs.mkdtempSync(path.join(os.tmpdir(), "ove-e2e-"));
const home = path.join(work, "home");
const dialogs = path.join(work, "dialogs");
for (const d of [home, path.join(work, "library"), dialogs]) fs.mkdirSync(d, { recursive: true });
// The app reports canonical paths (no 8.3 short names on Windows, no symlinked /tmp on macOS).
const library = fs.realpathSync.native(path.join(work, "library"));
const models = process.env.OVE_WHISPER_MODEL_DIR ?? path.join(os.tmpdir(), "ove-e2e-models");
fs.mkdirSync(models, { recursive: true });

const mock = await startMockMessages({ bin: MOCK, logFile: path.join(OUT, "mock-messages.log") });

// What the app sees. A fresh library and, on Linux, a fresh home (config, web storage, logs). Windows keeps the
// profile folders (they are not read from the environment there); CI runners start clean.
const env = {
  ...process.env,
  OVE_LIBRARY: library,
  OVE_E2E_DIALOGS: dialogs,
  OVE_WHISPER_MODEL_DIR: models,
  OVE_ANTHROPIC_BASE_URL: mock.url,
  // The assistant and the generation panel read keys from the environment when the credential store has none.
  ANTHROPIC_API_KEY: "e2e-demo-key-not-real-0123456789",
  FAL_KEY: "e2e-fal-key-not-real-0123456789",
  OVE_LOG: process.env.OVE_LOG ?? "info",
  RUST_BACKTRACE: "1",
};
if (!WINDOWS) {
  Object.assign(env, {
    HOME: home,
    XDG_CONFIG_HOME: path.join(home, ".config"),
    XDG_DATA_HOME: path.join(home, ".local", "share"),
    XDG_CACHE_HOME: path.join(home, ".cache"),
  });
}

const driverPort = await freePort();
const driver = await startTauriDriver({
  bin: TAURI_DRIVER,
  nativeDriver: process.env.OVE_E2E_NATIVE_DRIVER,
  port: driverPort,
  env,
  logFile: path.join(OUT, "tauri-driver.log"),
});

const media = {
  video: path.join(MEDIA, "h264_longgop.mp4"),
  video2: path.join(MEDIA, "vfr_phone.mp4"),
  audio: path.join(MEDIA, "audio_pcm.wav"),
};

let logDir = null;
let logSeen;
const newSession = () => new AppSession({ server: `http://127.0.0.1:${driverPort}/`, application: APP, log });

async function waitAppExit(ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (!appRunning(APP_EXE)) return true;
    await sleep(200);
  }
  return false;
}

const ctx = {
  app: null,
  log,
  work,
  library,
  dialogs,
  media,
  out: OUT,
  /** Closes the app as the person would (window closed, normal shutdown), then starts it again. */
  async restart() {
    await closeApp();
    ctx.app = await newSession().start();
  },
  /** Kills the app process outright (no shutdown code runs). */
  async kill() {
    killApp(APP_EXE);
    if (!(await waitAppExit(10000))) throw new Error("the app survived the kill");
    await ctx.app.abandon();
  },
  async relaunch() {
    ctx.app = await newSession().start();
  },
};

/** Closes the window as the person would; the app shuts down normally (final save, lock released). */
async function closeApp() {
  if (!closeAppWindow(APP_EXE, "OpenVideoEditor")) throw new Error("no app window to close");
  if (!(await waitAppExit(20000))) throw new Error("the app did not exit after its window closed");
  await ctx.app.abandon();
}

function copyAppLog(to) {
  if (!logDir || !fs.existsSync(logDir)) return;
  for (const f of fs.readdirSync(logDir).filter((n) => n.endsWith(".log"))) fs.copyFileSync(path.join(logDir, f), path.join(to, f));
}

/** Checks the page for error cards, error toasts and console errors since the last check. */
async function pageProblems(app) {
  if (!app?.alive) return [];
  const e = await app.takePageEvents();
  return [
    ...e.regionErrors.map((r) => `error card: ${r}`),
    ...e.toasts.filter((t) => t.code !== "info").map((t) => `error toast (${t.code}): ${t.text}`),
    ...e.console.map((c) => `console error: ${c}`),
  ];
}

const results = [];
// Launches in a row that the driver could not turn into a session. After two, the rest would fail the same way
// (a minute each), so they are reported as not run.
let launchFailures = 0;
const selected = FLOWS.filter((f) => !only || only.some((o) => f.name.toLowerCase().includes(o)));
log(`app: ${APP}\nwork: ${work}\nartefacts: ${OUT}\n`);
try {
  for (const [i, flow] of selected.entries()) {
    const id = String(i + 1).padStart(2, "0");
    const t0 = Date.now();
    log(`▶ ${id} ${flow.name}`);
    if (launchFailures >= 2) {
      log(`✗ ${id} NOT RUN: the driver could not start the app`);
      results.push({ flow: flow.name, ok: false, seconds: 0, error: "not run: the driver could not start the app" });
      continue;
    }
    let error = null;
    try {
      if (!ctx.app?.alive) {
        if (appRunning(APP_EXE)) killApp(APP_EXE);
        try {
          ctx.app = await newSession().start({ dismissWelcome: !flow.firstRun });
          launchFailures = 0;
        } catch (e) {
          launchFailures += 1;
          throw e;
        }
      }
      logDir = ctx.app.info?.logDir ?? logDir;
      await pageProblems(ctx.app); // what came before this flow is not its fault
      logSeen = logRecords(logDir).length;
      const timeout = flow.timeoutMs ?? 3 * 60 * 1000;
      let timer;
      await Promise.race([
        flow.run(ctx),
        new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error(`flow timed out after ${timeout / 1000} s`)), timeout);
        }),
      ]).finally(() => clearTimeout(timer));
      const problems = await pageProblems(ctx.app);
      await sleep(300); // the log writer is asynchronous
      problems.push(...badLogRecords(logRecords(logDir).slice(logSeen)).map((r) => `app log: ${r.slice(0, 600)}`));
      if (problems.length) throw new Error(problems.join("\n"));
    } catch (e) {
      error = e;
    }
    const seconds = (Date.now() - t0) / 1000;
    if (error) {
      const dir = path.join(OUT, `${id}-failed`);
      fs.mkdirSync(dir, { recursive: true });
      const saved = ctx.app ? await ctx.app.saveArtefacts(dir, "window").catch(() => []) : [];
      try {
        const extra = await pageProblems(ctx.app);
        if (extra.length) fs.writeFileSync(path.join(dir, "page-problems.txt"), extra.join("\n"));
      } catch {
        // the page may be gone
      }
      copyAppLog(dir);
      fs.writeFileSync(path.join(dir, "error.txt"), `${flow.name}\n\n${error.stack ?? error}`);
      log(`✗ ${id} FAILED (${seconds.toFixed(1)} s): ${error.message}\n  artefacts: ${dir} (${saved.length} captures)`);
      // A fresh app for the next flow.
      if (ctx.app) await ctx.app.abandon();
      if (appRunning(APP_EXE)) killApp(APP_EXE);
      await waitAppExit(10000);
    } else log(`✓ ${id} passed (${seconds.toFixed(1)} s)`);
    results.push({ flow: flow.name, ok: !error, seconds, error: error ? String(error.message) : undefined });
  }

  // Whole-run checks: the log has no ERROR or panic lines (other than the allow-listed GPU probe messages), and no
  // crash report was written.
  const t0 = Date.now();
  const name = "log: no ERROR or panic lines and no crash reports during the run";
  log(`▶ ${name}`);
  let error = null;
  try {
    if (ctx.app?.alive) await closeApp();
    assertLog();
  } catch (e) {
    error = e;
  }
  log(error ? `✗ FAILED: ${error.message}` : `✓ passed`);
  results.push({ flow: name, ok: !error, seconds: (Date.now() - t0) / 1000, error: error ? String(error.message) : undefined });
} finally {
  if (ctx.app?.alive) await ctx.app.quit();
  stop(driver);
  stop(mock.child);
  if (appRunning(APP_EXE)) killApp(APP_EXE);
  copyAppLog(OUT);
  fs.writeFileSync(path.join(OUT, "summary.json"), JSON.stringify({ app: APP, results, allowedLogErrors: ALLOWED_ERRORS.map((a) => a.why) }, null, 2));
  if (!keep) fs.rmSync(work, { recursive: true, force: true });
}

function assertLog() {
  if (!logDir) throw new Error("the app never reported its log folder");
  const records = logRecords(logDir);
  if (records.length === 0) throw new Error(`no log records in ${logDir}`);
  const bad = badLogRecords(records);
  const crashes = path.join(logDir, "crash-reports");
  const reports = fs.existsSync(crashes) ? fs.readdirSync(crashes) : [];
  const problems = [...bad.map((r) => `log: ${r.slice(0, 600)}`), ...reports.map((r) => `crash report: ${r}`)];
  if (problems.length) throw new Error(problems.join("\n"));
}

const failed = results.filter((r) => !r.ok);
log(`\n${results.length - failed.length}/${results.length} passed`);
for (const r of results) log(`${r.ok ? "  ok  " : "  FAIL"} ${r.flow}`);
process.exit(failed.length ? 1 : 0);
