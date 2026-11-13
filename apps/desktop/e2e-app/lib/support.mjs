// Processes and files around the app: tauri-driver, the mock Messages API, test media, the app's log, bundles.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const WINDOWS = process.platform === "win32";
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** A free loopback port. */
export function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.unref();
    s.on("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

async function waitForPort(port, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const ok = await new Promise((r) => {
      const c = net.connect(port, "127.0.0.1");
      c.on("connect", () => {
        c.destroy();
        r(true);
      });
      c.on("error", () => r(false));
    });
    if (ok) return;
    await sleep(200);
  }
  throw new Error(`nothing listens on port ${port} after ${ms} ms`);
}

/** Starts a long-running helper whose output goes to `logFile`. */
function startLogged(cmd, args, env, logFile) {
  const out = fs.openSync(logFile, "a");
  const child = spawn(cmd, args, { env, stdio: ["ignore", out, out], windowsHide: true });
  child.on("error", (e) => fs.appendFileSync(logFile, `\n[e2e] could not start ${cmd}: ${e.message}\n`));
  return child;
}

/** tauri-driver on `port`; the app it launches inherits `env`. */
export async function startTauriDriver({ bin, nativeDriver, port, env, logFile }) {
  const args = ["--port", String(port), "--native-port", String(await freePort())];
  if (nativeDriver) args.push("--native-driver", nativeDriver);
  const child = startLogged(bin, args, env, logFile);
  await waitForPort(port, 20000);
  return child;
}

/** The scripted Anthropic Messages API (`cargo build -p ove-agent --example mock_server`). */
export async function startMockMessages({ bin, logFile }) {
  const port = await freePort();
  const child = startLogged(bin, [`127.0.0.1:${port}`], process.env, logFile);
  await waitForPort(port, 20000);
  return { child, url: `http://127.0.0.1:${port}` };
}

export function stop(child) {
  if (child && child.exitCode === null) {
    try {
      child.kill();
    } catch {
      // already gone
    }
  }
}

/** Kills every running app process at once, as a crash or power cut would (no shutdown code runs). */
export function killApp(exeName) {
  if (WINDOWS) spawnSync("taskkill", ["/F", "/T", "/IM", exeName], { stdio: "ignore" });
  else spawnSync("pkill", ["-KILL", "-x", exeName.slice(0, 15)], { stdio: "ignore" });
}

/**
 * Closes the app's window the way the person does (the title bar's close button), so the app shuts down normally.
 * Linux: WM_DELETE_WINDOW through X11 (python3-xlib). Windows: WM_CLOSE to the main window. Returns whether a
 * window was asked to close.
 */
export function closeAppWindow(exeName, title) {
  if (WINDOWS) {
    const name = exeName.replace(/\.exe$/i, "");
    const ps = `$n = 0; Get-Process -Name '${name}' -ErrorAction SilentlyContinue | ForEach-Object { if ($_.CloseMainWindow()) { $n++ } }; if ($n -eq 0) { exit 1 }`;
    return spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", ps], { stdio: "inherit" }).status === 0;
  }
  const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "close_window.py");
  return spawnSync("python3", [script, title], { stdio: "inherit" }).status === 0;
}

/** Whether an app process is still running. */
export function appRunning(exeName) {
  if (WINDOWS) {
    const r = spawnSync("tasklist", ["/FI", `IMAGENAME eq ${exeName}`, "/NH"], { encoding: "utf8" });
    return (r.stdout ?? "").toLowerCase().includes(exeName.toLowerCase());
  }
  return spawnSync("pgrep", ["-x", exeName.slice(0, 15)], { stdio: "ignore" }).status === 0;
}

export function which(cmd) {
  const r = spawnSync(WINDOWS ? "where" : "which", [cmd], { encoding: "utf8" });
  return r.status === 0 ? r.stdout.split(/\r?\n/)[0].trim() : null;
}

/** Stream types and duration of a media file, or null when ffprobe is not available. */
export function ffprobe(file) {
  const bin = process.env.OVE_E2E_FFPROBE || which("ffprobe");
  if (!bin) return null;
  const out = execFileSync(bin, ["-v", "error", "-show_entries", "stream=codec_type,codec_name:format=duration", "-of", "json", file], {
    encoding: "utf8",
  });
  const j = JSON.parse(out);
  return {
    streams: (j.streams ?? []).map((s) => s.codec_type),
    codecs: (j.streams ?? []).map((s) => s.codec_name),
    duration: Number(j.format?.duration ?? 0),
  };
}

/**
 * A short spoken WAV for auto-captions: espeak-ng where it is installed (Linux CI installs it), else the Windows
 * speech synthesiser (System.Speech) through PowerShell, else the recording in fixtures/ (that sentence only).
 */
export function makeSpeech(file, text) {
  if (fs.existsSync(file) && fs.statSync(file).size > 1000) return file;
  const espeak = which("espeak-ng");
  if (espeak) {
    const r = spawnSync(espeak, ["-s", "140", "-w", file, text], { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(file)) return file;
  }
  if (WINDOWS) {
    const ps = [
      "Add-Type -AssemblyName System.Speech",
      "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer",
      `$s.SetOutputToWaveFile('${file.replace(/'/g, "''")}')`,
      `$s.Speak('${text.replace(/'/g, "''")}')`,
      "$s.Dispose()",
    ].join("; ");
    const r = spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", ps], { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(file) && fs.statSync(file).size > 1000) return file;
  }
  // Neither synthesiser (a Windows machine without an installed voice): the same sentence, recorded with espeak-ng.
  const recorded = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "fixtures", "speech.wav");
  if (fs.existsSync(recorded)) {
    fs.copyFileSync(recorded, file);
    return file;
  }
  return null;
}

// ---------------------------------------------------------------------------------------------------------------
// The app log

const RECORD_START = /^\S*\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d/;
const strip = (s) => s.replace(/\x1b\[[0-9;]*m/g, ""); // eslint-disable-line no-control-regex

/** Log records (a record is a timestamped line plus its continuation lines) from every `ove.*.log` in `dir`. */
export function logRecords(dir) {
  if (!dir || !fs.existsSync(dir)) return [];
  const records = [];
  for (const f of fs.readdirSync(dir).filter((n) => /^ove\..*log$/.test(n)).sort()) {
    for (const raw of fs.readFileSync(path.join(dir, f), "utf8").split(/\r?\n/)) {
      const line = strip(raw);
      if (RECORD_START.test(line) || records.length === 0) records.push(line);
      else records[records.length - 1] += `\n${line}`;
    }
  }
  return records;
}

/**
 * Known benign ERROR records. Each entry says why it is allowed.
 * - wgpu probing GPU drivers on Linux under Xvfb: the Vulkan loader lists ICDs that are not installed and EGL has no
 *   DRI driver. The app then uses the GL / software adapter (llvmpipe); nothing is wrong with the app.
 */
export const ALLOWED_ERRORS = [
  { re: / ERROR wgpu_hal::vulkan::instance:/, why: "Vulkan loader probing missing GPU drivers (Linux CI, Xvfb)" },
  { re: / ERROR wgpu_hal::gles::egl:/, why: "EGL without a DRI driver (Linux CI, Xvfb)" },
];

/** ERROR and panic records in the log that are not allow-listed. */
export function badLogRecords(records) {
  return records.filter(
    (r) => (/ ERROR /.test(r.split("\n")[0]) || /panicked|panic:/.test(r)) && !ALLOWED_ERRORS.some((a) => a.re.test(r)),
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Project bundles

export function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

/** Revision records in a bundle's `.revs/` folder. */
export function revisionFiles(bundle) {
  const dir = path.join(bundle, ".revs");
  return fs.existsSync(dir) ? fs.readdirSync(dir).filter((n) => /^\d+\.json$/.test(n)) : [];
}

/** The items of `project.json` (an object keyed by id in the file, or an array in older files). */
export function bundleItems(bundle) {
  const doc = readJson(path.join(bundle, "project.json"));
  const items = doc.project?.timelineItems ?? doc.project?.items ?? [];
  return Array.isArray(items) ? items : Object.values(items);
}
