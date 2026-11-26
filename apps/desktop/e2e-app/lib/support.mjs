// Processes and files around the app: tauri-driver, the mock Messages API, test media, the app's log, bundles.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

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

/** A picture of the real screen (Windows only; elsewhere null). Not a check: kept with the artefacts. */
export function captureScreen(file) {
  if (!WINDOWS) return null;
  const ps = [
    "Add-Type -AssemblyName System.Windows.Forms, System.Drawing",
    "$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds",
    "$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height",
    "[System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)",
    `$bmp.Save('${file.replace(/'/g, "''")}')`,
  ].join("; ");
  const r = spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", ps], { stdio: "ignore" });
  return r.status === 0 && fs.existsSync(file) ? file : null;
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
  // The panic record and the worker's "background task failed" record both name the forced panic.
  { re: /forced (worker|viewer) panic \(diagnostics\)/, why: "the resilience flow's deliberate panics" },
];

/** Crash reports the resilience flow causes on purpose (its forced panics). */
export const DELIBERATE_CRASH = /Message: forced (worker|viewer) panic \(diagnostics\)/;

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

/**
 * Mean SSIM (all planes) of the video of two files over the time window [from, to) seconds of each, measured with
 * a full FFmpeg CLI (`OVE_FIXTURE_FFMPEG`, else `ffmpeg` on PATH; the app's LGPL build has no filters for it).
 * Throws when no such CLI is available: a skipped comparison must not pass.
 */
export function ssim(a, b, from, to) {
  const bin = process.env.OVE_FIXTURE_FFMPEG || which("ffmpeg");
  if (!bin) throw new Error("SSIM needs a full ffmpeg CLI: set OVE_FIXTURE_FFMPEG");
  const cut = (i, label) => `[${i}:v]trim=start=${from}:end=${to},setpts=PTS-STARTPTS,format=yuv420p[${label}]`;
  const graph = `${cut(0, "a")};${cut(1, "b")};[a][b]ssim`;
  const r = spawnSync(bin, ["-hide_banner", "-nostats", "-i", a, "-i", b, "-filter_complex", graph, "-f", "null", "-"], {
    encoding: "utf8",
  });
  const m = /SSIM .* All:([0-9.]+)/.exec(r.stderr ?? "");
  if (r.status !== 0 || !m) throw new Error(`ffmpeg ssim failed (${r.status}): ${(r.stderr ?? "").slice(-600)}`);
  return Number(m[1]);
}

/** A valid RGB PNG of one colour (`w`×`h`), made without an image library. */
export function solidPng(w, h, [r, g, b]) {
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(zlib.crc32(body));
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr.set([8, 2, 0, 0, 0], 8); // 8-bit RGB, no interlace
  const row = Buffer.concat([Buffer.from([0]), Buffer.from(Array.from({ length: w }, () => [r, g, b]).flat())]);
  const raw = Buffer.concat(Array.from({ length: h }, () => row));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/**
 * A local stand-in for the fal.ai queue API (no key leaves the machine, nothing is spent). Every POST is a new
 * submission; the request stays IN_PROGRESS until `finish()`, then its result is one PNG image.
 */
export async function startFalStandIn() {
  const png = solidPng(64, 36, [40, 120, 200]);
  let posts = 0;
  let statusPolls = 0;
  let finished = false;
  const server = http.createServer((req, res) => {
    const base = `http://127.0.0.1:${server.address().port}`;
    const json = (v) => {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify(v));
    };
    req.resume();
    req.on("end", () => {
      if (req.method === "POST") {
        posts += 1;
        json({ request_id: `req${posts}`, status_url: `${base}/q/r/status`, response_url: `${base}/q/r`, cancel_url: `${base}/q/r/cancel` });
      } else if (req.url === "/q/r/status") {
        statusPolls += 1;
        json({ status: finished ? "COMPLETED" : "IN_PROGRESS" });
      } else if (req.url === "/q/r") {
        json({ images: [{ url: `${base}/out.png`, width: 64, height: 36, content_type: "image/png" }], seed: 1 });
      } else if (req.url === "/out.png") {
        res.writeHead(200, { "Content-Type": "image/png" });
        res.end(png);
      } else if (req.method === "PUT") {
        json({});
      } else {
        res.writeHead(404);
        res.end("{}");
      }
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    posts: () => posts,
    statusPolls: () => statusPolls,
    finish: () => {
      finished = true;
    },
    close: () => server.close(),
  };
}
