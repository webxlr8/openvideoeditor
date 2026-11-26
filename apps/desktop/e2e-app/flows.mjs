// The user flows. Each flow starts on Home in a running app, makes its own project, and checks real outcomes: the
// project the native side holds (through the UI's own bridge), files in the bundle, files written by exports.
// The first flow is the exception: it starts where a new person does, on the welcome.
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import readline from "node:readline";

import { By, Key, sleep } from "./lib/session.mjs";
import { WINDOWS, bundleItems, captureScreen, ffprobe, logRecords, makeSpeech, revisionFiles, ssim, which } from "./lib/support.mjs";

// ---------------------------------------------------------------------------------------------------------------
// Shared steps

async function goHome(app) {
  if (await app.exists("go-home")) await app.click("go-home");
  await app.find("home");
}

/** Blank project from Home, renamed to `name` in the project bar. Returns the native project view. */
async function newProject(app, name) {
  await goHome(app);
  await app.click("nav-home");
  await app.click("home-blank");
  await app.find("editor");
  await app.click("project-name");
  const input = await app.find("input.project-name-input");
  await app.replaceText(input, name);
  await input.sendKeys(Key.ENTER);
  await app.waitFor(async () => (await app.text("project-name")) === name, `project renamed to ${name}`);
  const view = await app.waitFor(async () => {
    const p = await app.project();
    return p && p.name === name ? p : null;
  }, "the native project to have the new name");
  return view;
}

/** Imports files with the Media panel's Import button; the native open dialog is answered by OVE_E2E_DIALOGS. */
async function importWithButton(ctx, files) {
  const { app } = ctx;
  fs.writeFileSync(path.join(ctx.dialogs, "open.txt"), files.join("\n"));
  const before = (await app.project()).media.length;
  if (!(await app.exists("import"))) await app.click("rail-media");
  await app.click("import");
  const view = await app.waitFor(async () => {
    const p = await app.project();
    return p.media.length >= before + files.length ? p : null;
  }, `${files.length} file(s) to be imported`, 60000);
  fs.rmSync(path.join(ctx.dialogs, "open.txt"), { force: true });
  await app.waitFor(async () => (await app.count("clip")) === view.items.length, "the new clips on the timeline");
  return view;
}

const itemsOn = (view, trackId) => view.items.filter((i) => i.trackId === trackId).sort((a, b) => a.start - b.start);
const mainClips = (view) => itemsOn(view, "t_main");

/** Clicks a timeline clip at a fraction of its width (selects it and moves the playhead there). */
async function clickClip(app, id, fraction = 0.5) {
  const el = await app.find(`[data-testid="clip"][data-id="${id}"]`);
  const r = await app.rect(el);
  await app.clickAt(el, Math.round(r.width * (fraction - 0.5)), 0);
  await app.waitFor(
    async () => (await el.getAttribute("class")).includes("selected"),
    `clip ${id} to be selected`,
  );
  return el;
}

async function detail(app, id) {
  return JSON.parse(await app.invoke("item_detail", { id }));
}

/** Pixels in the viewer canvas that are not black, out of a sample grid. */
async function litViewerPixels(app) {
  return app.driver.executeScript(`
    const c = document.querySelector('[data-testid="viewer-canvas"]');
    if (!c || !c.width || !c.height) return -1;
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let lit = 0;
    for (let y = 0; y < c.height; y += 8) for (let x = 0; x < c.width; x += 8) {
      const i = (y * c.width + x) * 4;
      if (d[i] + d[i + 1] + d[i + 2] > 30) lit++;
    }
    return lit;`);
}

/**
 * After `act` (something that moves the playhead or changes the picture) the viewer presents a new frame and, where
 * frames are streamed into the page, the canvas shows a real picture, not black.
 */
async function expectViewerFrame(ctx, act) {
  const { app } = ctx;
  const presented = async () => (await app.invoke("diagnostics")).viewer.presentedFrames;
  const before = await presented();
  await act();
  await app.waitFor(async () => (await presented()) > before, "the viewer to present a frame", 30000);
  // Asked now, not at launch: the native viewer attaches some seconds after the window appears.
  const backend = (await app.invoke("app_info")).viewerBackend;
  if (backend === "stream") {
    await app.waitFor(async () => (await litViewerPixels(app)) > 20, "a rendered frame in the viewer canvas", 30000);
  } else {
    // WebDriver cannot see a native surface under the page (the release workflow checks it on the real screen,
    // tools/preview-check.sh).
    ctx.log(`    native viewer (${backend}): checked through the presented-frame count`);
  }
}

/** Whether a step of the getting-started row is ticked. */
async function stepDone(app, step) {
  return (await (await app.find(`checklist-step-${step}`)).getAttribute("data-done")) === "true";
}

/** Every file named `name` under `dir`. */
function filesNamed(dir, name) {
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir, { recursive: true })
    .map(String)
    .filter((f) => path.basename(f) === name)
    .map((f) => path.join(dir, f));
}

/** Opens a project from Home's project grid by name. */
async function openFromHome(app, name) {
  await goHome(app);
  await app.click("nav-projects");
  await app.waitFor(async () => {
    for (const card of await app.all("project-card")) {
      if ((await app.textOf(card)).includes(name)) {
        await card.click();
        return true;
      }
    }
    return false;
  }, `a project card named ${name}`);
  await app.find("editor");
  return app.waitFor(async () => {
    const p = await app.project();
    return p && p.name === name ? p : null;
  }, `${name} to open`);
}

/** The part of a project that must survive a restart: tracks, items and their timing. */
function shape(view) {
  return {
    name: view.name,
    rev: view.rev,
    items: view.items
      .map((i) => ({ id: i.id, track: i.trackId, kind: i.kind, start: i.start, end: i.end, trim: i.trimStart, text: i.text }))
      .sort((a, b) => a.id.localeCompare(b.id)),
  };
}

/**
 * An `ove mcp` session on the app's library, spoken to over stdio (JSON-RPC, one message per line), as an external
 * agent would. `request` returns the result; `call` runs a tool and returns its result.
 */
function startMcp(ctx, args, env = {}) {
  assert.ok(fs.existsSync(ctx.ove), `the ove command line is needed: ${ctx.ove} (cargo build -p ove-cli)`);
  const child = spawn(ctx.ove, ["--library", ctx.library, "mcp", ...args], { env: { ...ctx.env, ...env }, stdio: ["pipe", "pipe", "ignore"] });
  const lines = readline.createInterface({ input: child.stdout });
  const waiting = [];
  lines.on("line", (l) => waiting.shift()?.(l));
  let id = 0;
  const request = (method, params) =>
    new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`no reply to ${method}`)), 60000);
      waiting.push((l) => {
        clearTimeout(timer);
        const m = JSON.parse(l);
        if (m.error) reject(new Error(`${method}: ${JSON.stringify(m.error)}`));
        else resolve(m.result);
      });
      id += 1;
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  return {
    request,
    call: (name, args) => request("tools/call", { name, arguments: args }),
    close: () => {
      child.stdin.end();
      child.kill();
    },
  };
}

// ---------------------------------------------------------------------------------------------------------------
// Export queue (extend-export-formats-and-queue): presets, the Exports panel, range renders

/** Streams of a media file as ffprobe reports them (codec, profile, pixel and sample formats), plus its duration. */
function probeFile(file) {
  const bin = process.env.OVE_E2E_FFPROBE || which("ffprobe");
  if (!bin) return null;
  const out = execFileSync(
    bin,
    [
      "-v", "error",
      "-show_entries",
      "stream=codec_type,codec_name,profile,pix_fmt,width,height,sample_fmt,sample_rate,bits_per_raw_sample,bits_per_sample:format=duration,format_name",
      "-of", "json", file,
    ],
    { encoding: "utf8" },
  );
  const j = JSON.parse(out);
  const streams = j.streams ?? [];
  return {
    video: streams.find((s) => s.codec_type === "video") ?? null,
    audio: streams.find((s) => s.codec_type === "audio") ?? null,
    duration: Number(j.format?.duration ?? 0),
    format: j.format?.format_name ?? "",
  };
}

/** Decodes every frame and sample with a full FFmpeg (as a player would); returns the errors it printed. */
function decodeErrors(file) {
  const bin = process.env.OVE_FIXTURE_FFMPEG || which("ffmpeg");
  if (!bin) return null;
  const r = spawnSync(bin, ["-v", "error", "-i", file, "-f", "null", "-"], { encoding: "utf8" });
  return r.status === 0 ? (r.stderr ?? "").trim() : `exit ${r.status}: ${(r.stderr ?? "").slice(-400)}`;
}

/** Integrated loudness (LUFS) of a file, measured with a full FFmpeg's EBU R128 filter. */
function integratedLoudness(file) {
  const bin = process.env.OVE_FIXTURE_FFMPEG || which("ffmpeg");
  if (!bin) return null;
  const r = spawnSync(bin, ["-hide_banner", "-nostats", "-i", file, "-af", "ebur128", "-f", "null", "-"], { encoding: "utf8" });
  const m = [...(r.stderr ?? "").matchAll(/I:\s+(-?[\d.]+) LUFS/g)].pop();
  return m ? Number(m[1]) : null;
}

/** Opens the export dialog with its presets loaded. */
async function openExportDialog(app) {
  await app.click("open-export");
  await app.find("export-dialog");
  await app.find("export-presets", 30000);
}

/** The dialog's summary of the current settings, once checked. */
async function exportSummary(app) {
  return app.waitFor(async () => {
    if (await app.exists("export-invalid")) throw new Error(`refused: ${await app.text("export-invalid")}`);
    return (await app.exists("export-summary")) ? (await app.text("export-summary")) || null : null;
  }, "the export settings to be checked");
}

/** Exports the open project with a preset through the dialog; waits for the job to finish and returns the file
 * (moved to the artefacts folder as `name`), the dialog's result line and the summary. */
async function exportWithPreset(ctx, project, preset, ext, name, timeout = 6 * 60 * 1000) {
  const { app } = ctx;
  await openExportDialog(app);
  await app.click(`export-preset-${preset}`);
  const summary = await exportSummary(app);
  await app.click("export-start");
  const done = await app.find("export-done", timeout);
  const line = (await app.textOf(done)).split("\n")[0];
  await app.clickText("Done", "export-dialog");
  await app.waitGone("export-dialog");
  const written = path.join(ctx.dialogs, "save", `${project}.${ext}`);
  assert.ok(fs.existsSync(written), `${written} exists`);
  const to = path.join(ctx.out, name);
  fs.renameSync(written, to);
  return { file: to, line, summary };
}

/** Seconds an export took, from the dialog's or panel's result line ("… in 12.3 s"). */
const secondsIn = (line) => Number(/in ([\d.]+) s/.exec(line)?.[1] ?? NaN);

/** Opens the Exports panel from the rail and returns its job rows by status. */
async function exportRows(app) {
  if (!(await app.exists("exports-panel"))) await app.click("rail-exports");
  await app.find("exports-panel");
  const rows = [];
  for (const el of await app.all("export-job"))
    rows.push({ el, id: await el.getAttribute("data-id"), status: await el.getAttribute("data-status"), text: await app.textOf(el) });
  return rows;
}

/** Right-clicks the marked range in the ruler and picks a menu entry. */
async function rangeMenu(app, entry) {
  const range = await app.find("range");
  await app.scrollIntoView(range);
  await app.driver.actions({ async: true }).contextClick(range).perform();
  await app.click(`menu-${entry}`);
}

// ---------------------------------------------------------------------------------------------------------------
// Flows

export const FLOWS = [
  {
    name: "first run: the welcome's three screens, Try the sample, a picture in the viewer, Cut ticked by a split",
    // The runner leaves the welcome up for this flow.
    firstRun: true,
    async run(ctx) {
      let { app } = ctx;
      const clean = await app.exists("welcome");
      if (clean) {
        assert.equal((await app.invoke("onboarding_state")).welcome, "pending", "no onboarding state yet");
        ctx.log("    clean profile: the welcome is shown before Home");
      } else {
        // A profile that has run the app before (a developer's Windows machine): the same screens from Help.
        ctx.log("    not a clean profile: the welcome is reopened from Help");
        await goHome(app);
        await app.click("help");
        await app.click("help-welcome");
      }

      // Three screens, Skip on each, and nothing to type a key into.
      const noFields = async () =>
        assert.equal(
          await app.count('[data-testid="welcome"] input, [data-testid="welcome"] textarea'),
          0,
          "no key is asked for",
        );
      for (const [n, screen] of ["what", "assistant", "privacy"].entries()) {
        if (n > 0) await app.click("welcome-next");
        await app.find(`welcome-screen-${screen}`);
        await app.find("welcome-skip");
        await noFields();
        await sleep(250); // a painted frame for the capture
        await app.saveArtefacts(ctx.out, `welcome-${n + 1}-${screen}`);
      }
      const privacy = (await app.text("welcome-privacy")).replace(/\s+/g, " ");
      assert.match(privacy, /Media files stay on this computer/);
      assert.match(privacy, /timeline, frames of your video, sound measurements and transcripts go to that assistant's provider/);
      assert.match(privacy, /Nothing is sent when no assistant is chosen/);
      assert.equal(await app.count("welcome-next"), 0, "the third screen is the last");

      // Try the sample: the editor opens a project that already holds a cut, a title, captions and a transition.
      await app.click("welcome-sample");
      await app.find("editor");
      await app.waitGone("welcome");
      let view = await app.waitFor(async () => {
        const p = await app.project();
        return p && p.name.startsWith("Sample project") ? p : null;
      }, "the sample project to open", 30000);
      assert.equal(path.dirname(fs.realpathSync.native(view.bundle)), ctx.library, "the sample is a project in the library");
      const clip = path.join(view.bundle, "media", "sample-clip.mp4");
      assert.ok(fs.existsSync(clip) && fs.statSync(clip).size > 100000, "the project has its own copy of the clip");
      const [first, second] = mainClips(view);
      assert.equal(mainClips(view).length, 2, "a cut: two clips on Main");
      assert.equal(first.mediaId, second.mediaId, "both from the sample clip");
      assert.ok(second.trimStart > first.end - first.start, "part of the source is cut out");
      assert.ok(second.transitionIn > 0, "a transition at the cut");
      assert.equal(view.items.filter((i) => i.kind === "text").length, 1, "a title");
      assert.ok(view.items.filter((i) => i.kind === "caption").length >= 2, "captions");
      assert.equal(view.media[0].status, "online");
      await app.waitFor(async () => (await app.count("clip")) === view.items.length, "the sample's clips on the timeline");
      await app.waitFor(async () => (await app.project()).media[0].thumbnail, "a thumbnail of the sample clip", 30000);
      // The viewer shows the sample's picture.
      await expectViewerFrame(ctx, () => clickClip(app, first.id, 0.5));
      assert.equal(await app.count("region-error"), 0);
      await app.saveArtefacts(ctx.out, "sample-project");
      if (clean) assert.equal((await app.invoke("onboarding_state")).welcome, "finished");

      // The guided first edit: the row is there (reopened from Help when an earlier run closed it), and what the
      // sample already holds ticked nothing.
      if (!(await app.exists("checklist"))) {
        await app.click("help");
        await app.click("help-getting-started");
      }
      await app.find("checklist");
      if (clean) {
        for (const s of ["import", "cut", "title", "captions", "export"]) {
          assert.equal(await stepDone(app, s), false, `${s} is not ticked yet`);
        }
      }
      // Cut: split the second clip where it is clicked. Nothing else is pressed.
      await clickClip(app, second.id, 0.5);
      await app.click("split");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return mainClips(p).length === 3 ? p : null;
      }, "the split");
      await app.waitFor(() => stepDone(app, "cut"), "Cut to be ticked by the split");
      assert.ok((await app.invoke("onboarding_state")).stepsDone.includes("cut"), "the tick is in the native state");
      ctx.log(`    getting started: ${await app.text("checklist-count")} after the split`);

      // Closed, then back from Help with the same tick.
      await app.click("checklist-close");
      await app.waitGone("checklist");
      await app.click("help");
      await app.click("help-getting-started");
      await app.find("checklist");
      assert.equal(await stepDone(app, "cut"), true, "Cut is still ticked after closing and reopening");
      // Help also lists the shortcuts.
      await app.click("help");
      await app.click("help-shortcuts");
      await app.find("shortcuts-overlay");
      await app.keys(Key.ESCAPE);
      await app.waitGone("shortcuts-overlay");
      // Left closed: the other flows run as for a person who is past the first edit.
      await app.click("checklist-close");
      await app.waitGone("checklist");

      // The state is the app's own file, and the next launch does not show the welcome again.
      if (clean && !WINDOWS) {
        // The runner's fresh home (Windows keeps the real profile folders).
        const files = filesNamed(path.join(ctx.work, "home"), "onboarding.json");
        assert.equal(files.length, 1, `onboarding.json in the app's configuration folder: ${files}`);
        const saved = JSON.parse(fs.readFileSync(files[0], "utf8"));
        assert.equal(saved.welcome, "finished");
        assert.ok(saved.stepsDone.includes("cut") && saved.checklistClosed === true, JSON.stringify(saved));
      }
      await ctx.restart();
      app = ctx.app;
      assert.equal(app.welcomeShown, false, "the welcome does not return at the next launch");
      assert.equal(await app.count("checklist"), 0, "the closed checklist stays closed");
      assert.ok((await app.invoke("onboarding_state")).stepsDone.includes("cut"));
      // Home has the Help entry too, with the same four things.
      await app.click("nav-home");
      await app.click("help");
      for (const entry of ["getting-started", "welcome", "sample", "shortcuts"]) await app.find(`help-${entry}`);
      await sleep(250); // a painted frame for the capture
      await app.saveArtefacts(ctx.out, "home-help");
      await app.click("help-shortcuts");
      await app.find("bind-split"); // Settings, on Shortcuts
      // A second copy from Home: the first keeps its split.
      await app.click("nav-home");
      await app.click("home-sample");
      await app.find("editor");
      const copy = await app.waitFor(async () => {
        const p = await app.project();
        return p && p.bundle !== view.bundle && p.name.startsWith("Sample project") ? p : null;
      }, "a second copy of the sample", 30000);
      assert.equal(mainClips(copy).length, 2, "the second copy is the sample as shipped");
      const firstCopy = bundleItems(view.bundle).filter((i) => i.trackId === "t_main");
      assert.equal(firstCopy.length, 3, "the first copy keeps its split");
    },
  },

  {
    name: "home: renders; What's new and Send feedback open and close; project search",
    async run(ctx) {
      const { app } = ctx;
      await goHome(app);
      await app.find("home-prompt");
      await app.click("nav-changelog");
      await app.find("whats-new");
      assert.match(await app.text("whats-new-limitations"), /\S/, "known limitations are listed");
      await app.keys(Key.ESCAPE);
      await app.waitGone("whats-new");

      await app.click("nav-feedback");
      await app.find("feedback-dialog");
      await app.type("feedback-text", "Export stopped halfway.");
      await app.waitFor(async () => (await app.text("feedback-env")).includes("startup: app"), "the environment in the report");
      await app.keys(Key.ESCAPE);
      await app.waitGone("feedback-dialog");

      // Two projects to search between.
      for (const n of ["Search Alpha", "Search Beta"]) await newProject(app, n);
      await goHome(app);
      await app.type("home-search", "alpha");
      await app.find("projects-page");
      await app.waitFor(async () => {
        const cards = await app.all("project-card");
        const names = await Promise.all(cards.map((c) => app.textOf(c)));
        return names.length === 1 && names[0].includes("Search Alpha");
      }, "only Search Alpha to match 'alpha'");
      await app.type("home-search", "no such project zz");
      await app.find("search-empty");
      assert.equal(await app.count("project-card"), 0);
      await app.type("home-search", "search");
      await app.waitFor(async () => (await app.count("project-card")) === 2, "both projects to match 'search'");
    },
  },

  {
    name: "project: create a blank project; the editor opens and the bundle is on disk",
    async run(ctx) {
      const { app } = ctx;
      const view = await newProject(app, "Blank E2E");
      assert.equal(path.dirname(fs.realpathSync.native(view.bundle)), ctx.library, "the bundle is in the library");
      assert.ok(fs.existsSync(path.join(view.bundle, "project.json")), "project.json exists");
      assert.ok(view.tracks.length >= 2, "starter tracks");
      for (const t of view.tracks) await app.find(`track-${t.name}`);
      await app.find("viewer-area");
      await app.find("timeline");
      assert.equal(view.items.length, 0);
    },
  },

  {
    name: "hints: empty panels say what they are for and offer one action; a starter prompt fills the message box",
    async run(ctx) {
      const { app } = ctx;
      await newProject(app, "Hints E2E");
      // Captions in a project without captions (and, here, without media): what captions do, and one action.
      await app.click("rail-captions");
      assert.match(await app.text("hint-captions"), /Captions put the spoken words on screen/);
      assert.equal(await app.text("hint-captions-action"), "Import media");
      assert.equal(await app.count('[data-testid="hint-captions"] button'), 1, "one action");
      // Text: the action adds a title, and the hint goes.
      await app.click("rail-text");
      await app.find("hint-text");
      await app.saveArtefacts(ctx.out, "hint-text");
      await app.click("hint-text-action");
      await app.waitFor(async () => (await app.project()).items.some((i) => i.kind === "text"), "a title from the hint");
      await app.waitGone("hint-text");
      // Media: the action is the import (the native file picker is answered by OVE_E2E_DIALOGS).
      fs.writeFileSync(path.join(ctx.dialogs, "open.txt"), ctx.media.video);
      await app.click("rail-media");
      await app.click("hint-media-action");
      const view = await app.waitFor(async () => {
        const p = await app.project();
        return p.media.length === 1 ? p : null;
      }, "the import from the Media hint", 60000);
      fs.rmSync(path.join(ctx.dialogs, "open.txt"), { force: true });
      // A panel that works on a clip: with nothing selected it offers the first clip, and stays open.
      await app.keys(Key.ESCAPE);
      await app.click("rail-transitions");
      assert.equal(await app.text("hint-transitions-action"), "Select the first clip");
      await app.click("hint-transitions-action");
      const [clip] = mainClips(view);
      await app.waitFor(
        async () => (await (await app.find(`[data-testid="clip"][data-id="${clip.id}"]`)).getAttribute("class")).includes("selected"),
        "the first clip to be selected",
      );
      await app.find("panel-transitions");
      await app.waitGone("hint-transitions");

      // An empty chat: three examples; a tap puts the words in the message box and sends nothing.
      if (!(await app.exists("chat"))) await app.click("toggle-chat");
      await app.find("starters");
      assert.equal(await app.count('[data-testid^="starter-"].starter'), 3, "three examples");
      const example = await app.text("starter-1");
      await app.click("starter-1");
      const composer = await app.find("composer");
      await app.waitFor(async () => (await composer.getAttribute("value")) === example, "the example in the message box");
      await app.click("starter-what");
      assert.match(await app.text("starter-answer"), /one undo step/);
      await app.saveArtefacts(ctx.out, "starter-prompts");
      await sleep(500);
      assert.equal(await app.count("msg-user"), 0, "nothing was sent");
      assert.equal(await app.count("starters"), 1, "the chat is still empty");
      // The message box is left empty for the next flow's project.
      await app.replaceText(composer, "");
    },
  },

  {
    name: "media: Import button and window drop add clips with thumbnails",
    async run(ctx) {
      const { app, media } = ctx;
      // The Import button runs the real import command; only the native file picker is answered by the
      // OVE_E2E_DIALOGS hook (WebDriver cannot reach native dialogs). The second import goes through the route
      // a window drop takes (`open-paths-requested`, the event the native drop handler emits), because WebDriver
      // cannot drop OS files either.
      const created = await newProject(app, "Import E2E");
      const revsBefore = revisionFiles(created.bundle).length;
      let view = await importWithButton(ctx, [media.video]);
      assert.equal(mainClips(view).length, 1, "one clip on Main");
      await app.emit("open-paths-requested", { paths: [media.video2, media.audio] });
      view = await app.waitFor(async () => {
        const p = await app.project();
        return p.media.length === 3 ? p : null;
      }, "the dropped files to be imported", 60000);
      assert.equal(mainClips(view).length, 2, "the dropped video follows on Main");
      assert.ok(view.items.some((i) => i.kind === "audio"), "the WAV is on an audio track");
      await app.waitFor(async () => (await app.count("clip")) === view.items.length, "every item drawn on the timeline");
      // Thumbnails: every video clip on the timeline and its media card show one.
      await app.waitFor(async () => {
        const p = await app.project();
        return p.media.filter((m) => m.kind === "video").every((m) => m.thumbnail);
      }, "video thumbnails", 30000);
      for (const clip of mainClips(view)) {
        const el = await app.find(`[data-testid="clip"][data-id="${clip.id}"]`);
        await app.waitFor(async () => (await el.getCssValue("background-image")).includes("url("), `a thumbnail on clip ${clip.name}`);
      }
      // The imported clip is selected (its properties show); the Media panel lists the files.
      await app.click("rail-media");
      await app.waitFor(async () => (await app.count("media-card")) === 3, "three media cards");
      const imgs = await app.all('[data-testid="media-card"] img');
      assert.ok(imgs.length >= 2, `media cards with thumbnails: ${imgs.length}`);
      assert.ok(revisionFiles(view.bundle).length >= revsBefore + 2, "each import is a revision on disk");
      const tsx = fs.readFileSync(path.join(view.bundle, "timeline.tsx"), "utf8");
      assert.match(tsx, /h264_longgop/, "the projection on disk lists the imported clip");
    },
  },

  {
    name: "timeline: select, split, trim, move, duplicate, delete, undo/redo, History restore",
    async run(ctx) {
      const { app, media } = ctx;
      await newProject(app, "Timeline E2E");
      let view = await importWithButton(ctx, [media.video]);
      const [clip] = mainClips(view);
      const length = clip.end - clip.start;

      // Select + split at the middle (clicking a clip selects it and moves the playhead there).
      await clickClip(app, clip.id, 0.5);
      await app.find("inspector");
      await app.click("split");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return mainClips(p).length === 2 ? p : null;
      }, "the split");
      let [a, b] = mainClips(view);
      assert.ok(Math.abs(a.end - b.start) < 0.05, "the halves meet");
      assert.ok(Math.abs(a.end - (clip.start + length / 2)) < 0.3, `split where clicked (at ${a.end} s)`);
      assert.ok(Math.abs(b.end - a.start - length) < 0.05, "the halves cover the original clip");
      await app.waitFor(async () => (await app.count("clip")) === view.items.length, "both halves drawn");

      // Trim the second half's end edge to the left.
      await clickClip(app, b.id, 0.5);
      const handle = await app.find(`[data-testid="clip"][data-id="${b.id}"] [data-testid="trim-end"]`);
      await app.click("snap-toggle"); // no snapping, so the edge lands where it is dropped
      await app.drag(handle, -40, 0);
      view = await app.waitFor(async () => {
        const p = await app.project();
        const nb = p.items.find((i) => i.id === b.id);
        // The start stays: a grab that missed the handle would move the clip, which also brings its end in.
        return nb && nb.end < b.end - 0.1 && Math.abs(nb.start - b.start) < 1e-6 ? p : null;
      }, "the trim");
      b = view.items.find((i) => i.id === b.id);

      // Move it right.
      const startBefore = b.start;
      const lengthBefore = b.end - b.start;
      await app.drag(await app.find(`[data-testid="clip"][data-id="${b.id}"]`), 60, 0);
      view = await app.waitFor(async () => {
        const p = await app.project();
        const nb = p.items.find((i) => i.id === b.id);
        return nb && nb.start > startBefore + 0.1 ? p : null;
      }, "the move");
      b = view.items.find((i) => i.id === b.id);
      assert.ok(Math.abs(b.end - b.start - lengthBefore) < 0.05, "a move keeps the length");

      // Duplicate, then delete the copy.
      await clickClip(app, b.id, 0.5);
      await app.click("insp-duplicate");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return mainClips(p).length === 3 ? p : null;
      }, "the duplicate");
      const copy = mainClips(view).find((i) => i.id !== a.id && i.id !== b.id);
      await clickClip(app, copy.id, 0.5);
      await app.click("insp-delete");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return mainClips(p).length === 2 && !p.items.some((i) => i.id === copy.id) ? p : null;
      }, "the delete");
      await app.waitFor(async () => (await app.count("clip")) === view.items.length, "the timeline to redraw");

      // Undo brings the copy back, redo removes it again.
      await app.click("undo");
      await app.waitFor(async () => mainClips(await app.project()).length === 3, "undo of the delete");
      await app.waitFor(async () => (await app.count(`[data-testid="clip"][data-id="${copy.id}"]`)) === 1, "the copy drawn again");
      await app.click("redo");
      await app.waitFor(async () => mainClips(await app.project()).length === 2, "redo of the delete");

      // History lists the revisions; restoring to before the split gives back the single clip.
      await app.click("history");
      await app.find("history-list");
      const rows = await app.all("history-row");
      const texts = await Promise.all(rows.map((r) => app.textOf(r)));
      ctx.log(`    history: ${texts.map((t) => t.split("\n").slice(0, 2).join(" ")).join(" | ")}`);
      assert.ok(rows.length >= 7, `history rows: ${rows.length}`);
      const split = texts.findIndex((t) => /split/i.test(t));
      assert.ok(split >= 0, "the split is in the history");
      await rows[split].click();
      await app.click("history-restore");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return mainClips(p).length === 1 ? p : null;
      }, "the restore to before the split");
      const restored = mainClips(view)[0];
      assert.ok(Math.abs(restored.end - restored.start - length) < 0.05, "the original clip is back");
      if (await app.exists("history-panel")) await app.keys(Key.ESCAPE);
      if (await app.exists("history-panel")) await app.click('[data-testid="history-panel"] button[aria-label="Close"]');
      await app.click("snap-toggle");
    },
  },

  {
    name: "inspector: opacity, scale and volume persist after reopening the project",
    async run(ctx) {
      const { app, media } = ctx;
      await newProject(app, "Inspector E2E");
      const view = await importWithButton(ctx, [media.video]);
      const [clip] = mainClips(view);
      await clickClip(app, clip.id, 0.3);
      await app.find("inspector");
      await app.setRange("insp-opacity", 0.5);
      await app.waitFor(async () => (await detail(app, clip.id)).opacity === 0.5, "opacity 0.5");
      const scale = await app.find("insp-scale");
      await app.replaceText(scale, "1.5");
      await scale.sendKeys(Key.ENTER);
      await app.waitFor(async () => (await detail(app, clip.id)).transform?.scale === 1.5, "scale 1.5");
      await app.click("insp-tab-audio");
      await app.setRange("insp-volume", 50);
      await app.waitFor(async () => (await detail(app, clip.id)).volume === 50, "volume 50");

      // Home closes the project (final snapshot); reopening shows the same values.
      await goHome(app);
      const saved = bundleItems(view.bundle).find((i) => i.id === clip.id);
      assert.equal(saved.opacity, 0.5, "opacity in project.json");
      assert.equal(saved.volume, 50, "volume in project.json");
      assert.equal(saved.transform?.scale, 1.5, "scale in project.json");
      await openFromHome(app, "Inspector E2E");
      await clickClip(app, clip.id, 0.3);
      await app.find("inspector");
      await app.waitFor(async () => (await (await app.find("insp-opacity")).getAttribute("value")) === "0.5", "the opacity slider at 50%");
      assert.equal(await (await app.find("insp-scale")).getAttribute("value"), "1.5");
      await app.click("insp-tab-audio");
      await app.waitFor(async () => (await (await app.find("insp-volume")).getAttribute("value")) === "50", "the volume slider at 50%");
    },
  },

  {
    name: "text: add a title from the Text panel and edit its words",
    async run(ctx) {
      const { app } = ctx;
      await newProject(app, "Text E2E");
      await app.click("rail-text");
      await app.click("text-preset-headline");
      let view = await app.waitFor(async () => {
        const p = await app.project();
        return p.items.some((i) => i.kind === "text") ? p : null;
      }, "a title item");
      const title = view.items.find((i) => i.kind === "text");
      await clickClip(app, title.id, 0.5);
      await app.click("insp-tab-text");
      const area = await app.find("insp-text");
      await app.replaceText(area, "Hello from the e2e suite");
      await app.click("insp-tab-general"); // leaving the field commits it
      await app.waitFor(async () => {
        const p = await app.project();
        return p.items.find((i) => i.id === title.id)?.text === "Hello from the e2e suite" ? p : null;
      }, "the new title text");
      await app.waitFor(
        async () => (await app.textOf(await app.find(`[data-testid="clip"][data-id="${title.id}"]`))).includes("Hello from the e2e suite"),
        "the timeline label to show the new words",
      );
      assert.equal((await detail(app, title.id)).textContent, "Hello from the e2e suite");
    },
  },

  {
    name: "captions: auto-captions on a clip with speech add caption items",
    timeoutMs: 15 * 60 * 1000,
    async run(ctx) {
      const { app } = ctx;
      const wav = makeSpeech(path.join(ctx.work, "speech.wav"), "Hello and welcome. This video shows how captions are made on this computer.");
      assert.ok(wav, "a speech clip (needs espeak-ng, or the Windows speech synthesiser)");
      await newProject(app, "Captions E2E");
      await importWithButton(ctx, [wav]);
      await app.click("rail-captions");
      await app.find("caption-auto");
      // A model already on disk (OVE_WHISPER_MODEL_DIR cache) is used; else tiny downloads (77 MB).
      const models = await app.invoke("caption_models");
      const model = models.find((m) => m.installed)?.id ?? "tiny";
      ctx.log(`    speech model: ${model}${models.find((m) => m.id === model)?.installed ? " (cached)" : " (downloading)"}`);
      await app.select("caption-model", model);
      await app.click("caption-generate");
      const result = await app.find("caption-result", 14 * 60 * 1000);
      const text = await app.textOf(result);
      ctx.log(`    ${text}`);
      assert.match(text, /^Added \d+ caption/, text);
      const view = await app.project();
      const caps = view.items.filter((i) => i.kind === "caption");
      assert.ok(caps.length > 0, "caption items in the project");
      const words = caps.map((c) => c.text ?? "").join(" ").toLowerCase();
      assert.match(words, /welcome|captions|computer|video/, `recognised words: ${words}`);
      await app.waitFor(async () => (await app.count('[data-testid="clip"].kind-caption')) === caps.length, "caption items on the timeline");
    },
  },

  {
    name: "effects + transitions: one effect and one transition; the preview frame renders",
    async run(ctx) {
      const { app, media } = ctx;
      await newProject(app, "Fx E2E");
      let view = await importWithButton(ctx, [media.video, media.video2]);
      const [first, second] = mainClips(view);
      await clickClip(app, first.id, 0.5);
      await app.click("rail-effects");
      await app.click("effect-duotone-clip");
      await app.waitFor(async () => {
        const d = await detail(app, first.id);
        return (d.effects ?? []).some((e) => e.type === "duotone" || e.id === "duotone" || JSON.stringify(e).includes("duotone"));
      }, "the duotone effect on the first clip");
      await clickClip(app, first.id, 0.5);
      await app.click("rail-transitions");
      await app.click("transition-dissolve-between");
      await app.waitFor(async () => {
        const p = await app.project();
        return (p.items.find((i) => i.id === second.id)?.transitionIn ?? 0) > 0 ? p : null;
      }, "a dissolve between the clips");
      assert.equal((await detail(app, second.id)).transitionIn?.type, "dissolve");
      // The viewer shows the composited timeline at the cut, and no error card.
      await expectViewerFrame(ctx, () => clickClip(app, first.id, 0.9));
      // Windows: a capture of the real screen, kept with the artefacts.
      const shot = captureScreen(path.join(ctx.out, "viewer-screen.png"));
      if (shot) ctx.log(`    screen capture: ${shot}`);
      assert.equal(await app.count("region-error"), 0);
    },
  },

  {
    // fix-render-and-edit-correctness task 2.4.
    name: "split export: a sped-up clip with fades exports the same frames after a split (SSIM >= 0.99)",
    timeoutMs: 10 * 60 * 1000,
    async run(ctx) {
      const { app, media } = ctx;
      const project = "Split Export E2E";
      await newProject(app, project);
      let view = await importWithButton(ctx, [media.video]);
      const [clip] = mainClips(view);

      // Double speed from the Inspector's speed chips.
      await clickClip(app, clip.id, 0.5);
      await app.find("inspector");
      await app.clickText("2×", "inspector");
      await app.waitFor(async () => (await detail(app, clip.id)).playbackSpeed === 2, "speed 2×");
      // A fade at each edge from the Transitions panel (0.5 s). Selecting a clip opens the Inspector, so the panel
      // is opened after the selection.
      await app.click("rail-transitions");
      await app.setRange("transition-duration", 0.5);
      await app.clickText("In", "transition-fade");
      await app.waitFor(async () => (await detail(app, clip.id)).transitionIn?.type === "fade", "a fade in");
      await app.clickText("Out", "transition-fade");
      await app.waitFor(async () => (await detail(app, clip.id)).transitionOut?.type === "fade", "a fade out");
      view = await app.project();
      const sped = mainClips(view)[0];
      ctx.log(`    clip ${sped.start}–${sped.end} s at 2×, fades in and out`);

      const exportTo = async (name) => {
        await openExportDialog(app);
        await app.click("export-preset-social-720p");
        await exportSummary(app);
        await app.click("export-start");
        await app.find("export-done", 6 * 60 * 1000);
        await app.clickText("Done", "export-dialog");
        await app.waitGone("export-dialog");
        const written = path.join(ctx.dialogs, "save", `${project}.mp4`);
        assert.ok(fs.existsSync(written), `${written} exists`);
        const to = path.join(ctx.out, name);
        fs.renameSync(written, to);
        return to;
      };
      const uncut = await exportTo("split-uncut.mp4");

      // Split a quarter of the way in (the click moves the playhead there), where the source still moves on both
      // sides of the cut.
      await clickClip(app, sped.id, 0.25);
      await app.click("split");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return mainClips(p).length === 2 ? p : null;
      }, "the split");
      const [left, right] = mainClips(view);
      const at = left.end;
      assert.ok(Math.abs(at - right.start) < 1e-6, "the parts meet");
      const [dl, dr] = [await detail(app, left.id), await detail(app, right.id)];
      assert.equal(dl.transitionIn?.type, "fade", "the fade in stays on the left part");
      assert.equal(dl.transitionOut, undefined, "the left part has no fade out");
      assert.equal(dr.transitionIn, undefined, "the right part has no fade in");
      assert.equal(dr.transitionOut?.type, "fade", "the fade out moves to the right part");
      assert.equal(dr.playbackSpeed, 2, "the right part keeps the speed");
      const cut = await exportTo("split-cut.mp4");

      // The frames either side of the cut, and the whole export, match the uncut export.
      const near = ssim(uncut, cut, Math.max(0, at - 0.4), at + 0.4);
      const whole = ssim(uncut, cut, 0, sped.end);
      ctx.log(`    split at ${at.toFixed(3)} s: SSIM ${near.toFixed(5)} within 0.4 s of the cut, ${whole.toFixed(5)} overall`);
      assert.ok(near >= 0.99, `SSIM around the cut ${near}`);
      assert.ok(whole >= 0.99, `SSIM over the export ${whole}`);
    },
  },

  {
    name: "export: MP4 via the Export dialog; FCPXML with the 'what won't carry over' step",
    timeoutMs: 8 * 60 * 1000,
    async run(ctx) {
      const { app, media } = ctx;
      await newProject(app, "Export E2E");
      await importWithButton(ctx, [media.video]);
      await openExportDialog(app);
      await app.click("export-preset-social-720p");
      await exportSummary(app);
      await app.click("export-start");
      const done = await app.find("export-done", 6 * 60 * 1000);
      ctx.log(`    ${(await app.textOf(done)).split("\n")[0]}`);
      const mp4 = path.join(ctx.dialogs, "save", "Export E2E.mp4");
      assert.ok(fs.existsSync(mp4), `${mp4} exists`);
      assert.ok(fs.statSync(mp4).size > 10000, "the MP4 is not empty");
      const probe = ffprobe(mp4);
      if (probe) {
        ctx.log(`    ffprobe: ${probe.codecs.join("+")} ${probe.duration.toFixed(2)} s`);
        assert.ok(probe.streams.includes("video") && probe.streams.includes("audio"), `video and audio: ${probe.streams}`);
        assert.ok(Math.abs(probe.duration - 4) < 0.5, `duration ${probe.duration}`);
      } else ctx.log("    (no ffprobe: stream check skipped)");
      await app.clickText("Done", "export-dialog");
      await app.waitGone("export-dialog");

      // A title does not carry over to FCPXML: the dialog says so before writing.
      await app.click("rail-text");
      await app.click("text-preset-headline");
      await app.waitFor(async () => (await app.project()).items.some((i) => i.kind === "text"), "a title");
      await app.click("open-export");
      await app.click("export-kind-timeline");
      await app.find("timeline-formats");
      await app.clickText("FCPXML", "timeline-formats");
      await app.click("timeline-export-start");
      await app.find("timeline-report");
      const report = await app.text("timeline-report");
      ctx.log(`    ${report.split("\n").slice(1, -1).join("; ")}`);
      assert.match(report, /title styling/, "the title's styling is listed");
      await app.saveArtefacts(ctx.out, "export-timeline-report");
      await app.click("timeline-export-continue");
      await app.find("timeline-export-done");
      const fcpxml = path.join(ctx.dialogs, "save", "Export E2E.fcpxml");
      assert.ok(fs.existsSync(fcpxml), `${fcpxml} exists`);
      const xml = fs.readFileSync(fcpxml, "utf8");
      assert.match(xml, /<fcpxml/);
      assert.match(xml, /h264_longgop/);
      await app.clickText("Done", "export-dialog");
      await app.waitGone("export-dialog");
    },
  },

  {
    // extend-export-formats-and-queue task 6.1 (Windows half) and the encode speeds of 6.2.
    name: "export formats: ProRes 422 HQ, ProRes 4444 with alpha, H.264 1080p and 24-bit WAV from presets probe and decode",
    timeoutMs: 25 * 60 * 1000,
    async run(ctx) {
      const { app, media } = ctx;
      const project = "Export Formats E2E";
      await newProject(app, project);
      await importWithButton(ctx, [media.video]);

      // Presets this computer has no encoder for are listed but cannot be chosen.
      await openExportDialog(app);
      const off = [];
      for (const p of await app.all('[data-testid^="export-preset-"]')) {
        if (!(await p.isEnabled())) off.push(`${(await app.textOf(p)).split("\n")[0]} (${await p.getAttribute("title")})`);
      }
      ctx.log(`    not on this computer: ${off.join("; ") || "none"}`);
      await app.clickText("Close", "export-dialog");
      await app.waitGone("export-dialog");

      const cases = [
        {
          preset: "prores-422-hq-master",
          ext: "mov",
          ok: (p) =>
            p.video?.codec_name === "prores" && /HQ/i.test(p.video.profile) && p.video.pix_fmt === "yuv422p10le" &&
            p.video.width === 1920 && p.video.height === 1080 && p.audio?.codec_name === "pcm_s24le",
        },
        {
          preset: "prores-4444-with-alpha",
          ext: "mov",
          ok: (p) => p.video?.codec_name === "prores" && /4444/.test(p.video.profile) && /^yuva444p/.test(p.video.pix_fmt),
        },
        {
          preset: "youtube-1080p",
          ext: "mp4",
          ok: (p) => p.video?.codec_name === "h264" && p.video.height === 1080 && p.audio?.codec_name === "aac",
        },
        {
          preset: "audio-wav-24-bit",
          ext: "wav",
          ok: (p) => !p.video && p.audio?.codec_name === "pcm_s24le" && Number(p.audio.sample_rate) === 48000,
        },
      ];
      const speeds = [];
      for (const c of cases) {
        const r = await exportWithPreset(ctx, project, c.preset, c.ext, `formats-${c.preset}.${c.ext}`);
        const p = probeFile(r.file);
        assert.ok(p, "ffprobe is needed for this flow");
        const v = p.video ? `${p.video.codec_name} ${p.video.profile ?? ""} ${p.video.pix_fmt} ${p.video.width}x${p.video.height}` : "no video";
        const a = p.audio ? `${p.audio.codec_name} ${p.audio.sample_rate} Hz` : "no audio";
        const seconds = secondsIn(r.line);
        const speed = p.duration / seconds;
        speeds.push({ preset: c.preset, summary: r.summary, duration: p.duration, seconds, speed });
        ctx.log(`    ${c.preset}: ${v}; ${a}; ${p.duration.toFixed(2)} s in ${seconds} s (${speed.toFixed(2)}× realtime)`);
        assert.ok(c.ok(p), `${c.preset} probes as expected: ${JSON.stringify(p)}`);
        assert.ok(Math.abs(p.duration - 4) < 0.5, `${c.preset} duration ${p.duration}`);
        const errors = decodeErrors(r.file);
        assert.equal(errors, "", `${c.preset} decodes without errors`);
      }
      fs.writeFileSync(path.join(ctx.out, "export-speeds.json"), JSON.stringify(speeds, null, 2));
    },
  },

  {
    // extend-export-formats-and-queue tasks 5.1 and 6.2 (Windows half).
    name: "export queue: three exports, one cancelled in the Exports panel, a sound-only WAV at -14 LUFS; quit and reopen, all finish",
    timeoutMs: 25 * 60 * 1000,
    async run(ctx) {
      const { media } = ctx;
      const project = "Export Queue E2E";
      await newProject(ctx.app, project);
      await importWithButton(ctx, [media.video, media.video2]);
      const queue = async (setup) => {
        const app = ctx.app;
        await openExportDialog(app);
        await setup(app);
        const summary = await exportSummary(app);
        await app.click("export-start");
        await app.find("export-progress", 30000);
        await app.click("export-show-exports");
        await app.waitGone("export-dialog");
        return summary;
      };
      ctx.log(`    queued: ${await queue((app) => app.click("export-preset-prores-422-hq-master"))}`);
      ctx.log(`    queued: ${await queue((app) => app.click("export-preset-youtube-1080p"))}`);
      ctx.log(
        `    queued: ${await queue(async (app) => {
          await app.click("export-audio-only");
          await app.select("export-loudness", "-14");
        })}`,
      );
      const save = (ext) => path.join(ctx.dialogs, "save", `${project}.${ext}`);
      const ours = async () => {
        const rows = await exportRows(ctx.app);
        const by = (ext) => rows.find((r) => r.text.includes(`${project}.${ext}`));
        return { mov: by("mov"), mp4: by("mp4"), wav: by("wav") };
      };
      let rows = await ctx.app.waitFor(async () => {
        const r = await ours();
        return r.mov && r.mp4 && r.wav ? r : null;
      }, "the three exports in the Exports panel");
      ctx.log(`    panel: mov ${rows.mov.status}, mp4 ${rows.mp4.status}, wav ${rows.wav.status}`);
      assert.ok(["queued", "running"].includes(rows.mp4.status), `the second export waits or runs: ${rows.mp4.status}`);

      // Cancel the second one from the panel.
      const t0 = Date.now();
      await ctx.app.click(await rows.mp4.el.findElement(By.css('[data-testid="export-job-cancel"]')));
      rows = await ctx.app.waitFor(async () => {
        const r = await ours();
        return r.mp4?.status === "cancelled" ? r : null;
      }, "the second export to be cancelled", 10000);
      ctx.log(`    cancelled in the panel after ${Date.now() - t0} ms; mov ${rows.mov.status}, wav ${rows.wav.status}`);

      // Quit while the exports go on; they belong to the queue's worker, not to the window.
      const pending = Object.entries(rows).filter(([, r]) => ["queued", "running"].includes(r.status)).map(([k]) => k);
      ctx.log(`    quitting with ${pending.join(", ") || "nothing"} still to finish`);
      await ctx.restart();
      await openFromHome(ctx.app, project);
      rows = await ctx.app.waitFor(
        async () => {
          const r = await ours();
          return r.mov?.status === "done" && r.wav?.status === "done" ? r : null;
        },
        "the ProRes and WAV exports to finish after reopening",
        15 * 60 * 1000,
        1000,
      );
      assert.equal(rows.mp4.status, "cancelled", "the cancelled export stays cancelled");
      // A finished row: file and status, what was written, then the encoder and time taken.
      ctx.log(`    mov: ${rows.mov.text.split("\n")[2]}`);
      ctx.log(`    wav: ${rows.wav.text.split("\n")[2]}`);

      assert.ok(!fs.existsSync(save("mp4")), "the cancelled export left no file");
      for (const f of fs.readdirSync(path.join(ctx.dialogs, "save"))) assert.ok(!f.endsWith(".partial"), `no partial file: ${f}`);
      const mov = probeFile(save("mov"));
      assert.equal(mov.video?.codec_name, "prores");
      assert.equal(decodeErrors(save("mov")), "", "the ProRes file decodes");
      const wav = probeFile(save("wav"));
      assert.ok(!wav.video && wav.audio?.codec_name === "pcm_s24le", `sound-only 24-bit WAV: ${JSON.stringify(wav)}`);
      const lufs = integratedLoudness(save("wav"));
      ctx.log(`    WAV integrated loudness ${lufs} LUFS (target -14)`);
      assert.ok(lufs !== null && Math.abs(lufs + 14) <= 1, `loudness ${lufs}`);
      for (const ext of ["mov", "wav"]) fs.renameSync(save(ext), path.join(ctx.out, `queue.${ext}`));
    },
  },

  {
    // extend-export-formats-and-queue task 5.2 in the built app.
    name: "range: save a marked range as media, render it in place, one undo restores the clips",
    timeoutMs: 10 * 60 * 1000,
    async run(ctx) {
      const { app, media } = ctx;
      const project = "Range Media E2E";
      await newProject(app, project);
      let view = await importWithButton(ctx, [media.video]);
      const [clip] = mainClips(view);
      // X marks the selected clip as the range.
      await clickClip(app, clip.id, 0.5);
      await app.keys("x");
      const range = await app.find("range");
      const [a, b] = [Number(await range.getAttribute("data-in")), Number(await range.getAttribute("data-out"))];
      ctx.log(`    marked ${a.toFixed(3)}–${b.toFixed(3)} s of a ${clip.start}–${clip.end} s clip`);
      assert.ok(b > a, "a range is marked");

      const before = shape(await app.project());
      const mediaBefore = (await app.project()).media.length;
      await rangeMenu(app, "save-range");
      ctx.log(`    ${(await app.textOf(await app.find("range-done", 5 * 60 * 1000))).split("\n")[0]}`);
      await app.clickText("Done", "range-media-dialog");
      view = await app.waitFor(async () => {
        const p = await app.project();
        return p.media.length === mediaBefore + 1 ? p : null;
      }, "the rendered range in the media");
      const saved = view.media[view.media.length - 1];
      assert.deepEqual(shape(view).items, before.items, "saving as media leaves the clips as they were");
      const rendered = path.join(view.bundle, "media", "rendered");
      const files = fs.readdirSync(rendered);
      assert.equal(files.length, 1, `one rendered file: ${files}`);
      const p = probeFile(path.join(rendered, files[0]));
      ctx.log(`    ${files[0]}: ${p.video?.codec_name} ${p.video?.profile} ${p.video?.width}x${p.video?.height}, ${p.duration.toFixed(3)} s`);
      assert.ok(Math.abs(p.duration - (b - a)) < 0.1, `the file lasts the range: ${p.duration}`);
      assert.equal(p.video?.width, 1920);
      assert.equal(saved.status, "online");

      await rangeMenu(app, "render-in-place");
      await app.click("range-confirm");
      ctx.log(`    ${(await app.textOf(await app.find("range-done", 5 * 60 * 1000))).split("\n")[0]}`);
      await app.clickText("Done", "range-media-dialog");
      view = await app.waitFor(async () => {
        const v = await app.project();
        return v.items.some((i) => i.mediaId && i.mediaId !== clip.mediaId && Math.abs(i.start - a) < 1e-3) ? v : null;
      }, "a clip of the rendered range in place");
      const placed = view.items.find((i) => i.mediaId !== clip.mediaId && Math.abs(i.start - a) < 1e-3);
      ctx.log(`    in place: ${placed.trackId} ${placed.start.toFixed(3)}–${placed.end.toFixed(3)} s; main ${mainClips(view).map((i) => `${i.start.toFixed(2)}–${i.end.toFixed(2)}`).join(", ")}`);
      assert.ok(Math.abs(placed.end - b) < 1e-3, "the clip covers the range");
      for (const i of view.items.filter((x) => x.id !== placed.id && x.trackId !== "t_audio")) {
        assert.ok(i.end <= a + 1e-3 || i.start >= b - 1e-3, `nothing else covers the range: ${JSON.stringify(i)}`);
      }

      const placedRev = (await app.project()).rev;
      await app.click("undo");
      let last;
      try {
        await app.waitFor(async () => {
          last = await app.project();
          return JSON.stringify(shape(last).items) === JSON.stringify(before.items);
        }, "one undo to restore the clips");
      } catch (e) {
        const h = await app.invoke("project_history");
        ctx.log(`    history: ${h.slice(0, 6).map((r) => `${r.rev} ${r.source} ${r.summary}`).join(" | ")}`);
        ctx.log(`    placed at rev ${placedRev}; after undo rev ${last?.rev}: ${JSON.stringify(shape(last).items)}
    before: ${JSON.stringify(before.items)}`);
        throw e;
      }
      assert.ok(fs.readdirSync(rendered).length >= 1, "the rendered files stay in the bundle");
    },
  },

  {
    // extend-export-formats-and-queue task 6.3 (Windows half).
    name: "agent export: an MCP client queues an export and waits on job_status, cancels a second; the Exports panel shows both",
    timeoutMs: 10 * 60 * 1000,
    async run(ctx) {
      const { app, media } = ctx;
      const project = "Agent Export E2E";
      await newProject(app, project);
      await importWithButton(ctx, [media.video]);
      const mcp = startMcp(ctx, []);
      const out1 = path.join(ctx.out, "agent-1.mov");
      const out2 = path.join(ctx.out, "agent-2.mp4");
      let job1;
      let job2;
      try {
        await mcp.request("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e-agent", version: "1.0" } });
        const r1 = await mcp.call("export_video", { project, out: out1, preset: "ProRes 422 HQ master" });
        assert.equal(r1.isError, false, JSON.stringify(r1));
        job1 = r1.structuredContent.jobId;
        const r2 = await mcp.call("export_video", { project, out: out2, preset: "YouTube 1080p" });
        job2 = r2.structuredContent.jobId;
        ctx.log(`    queued ${job1} (${r1.structuredContent.status}) and ${job2} (${r2.structuredContent.status})`);
        const t0 = Date.now();
        const c = await mcp.call("cancel_job", { job_id: job2 });
        assert.equal(c.isError, false, JSON.stringify(c));
        assert.equal(c.structuredContent.status, "cancelled");
        ctx.log(`    cancel_job: ${c.structuredContent.status} in ${Date.now() - t0} ms`);
        const s = await mcp.call("job_status", { job_id: job1, wait: true });
        assert.equal(s.structuredContent.status, "done", JSON.stringify(s.structuredContent));
        ctx.log(`    job_status: ${s.content?.[0]?.text ?? s.structuredContent.status}`);
        const unknown = await mcp.call("job_status", { job_id: "x_0_00000000" });
        assert.equal(unknown.isError, true, "an unknown job id is an error");
        assert.match(JSON.stringify(unknown), /job\.unknown/);
      } finally {
        mcp.close();
      }
      assert.equal(probeFile(out1).video?.codec_name, "prores");
      assert.ok(!fs.existsSync(out2), "the cancelled job wrote nothing");
      // The app shows the agent's jobs in the same queue.
      const rows = await app.waitFor(async () => {
        const r = await exportRows(app);
        const one = r.find((x) => x.id === job1);
        const two = r.find((x) => x.id === job2);
        return one && two ? { one, two } : null;
      }, "the agent's jobs in the Exports panel");
      assert.equal(rows.one.status, "done");
      assert.equal(rows.two.status, "cancelled");
      assert.match(rows.one.text, /by e2e-agent/);
      ctx.log(`    panel: ${rows.one.text.split("\n")[1]}`);
    },
  },

  {
    name: "templates: required fields block the start; a filled template opens a project with the brief in the chat",
    async run(ctx) {
      const { app } = ctx;
      await goHome(app);
      await app.click("nav-templates");
      await app.click("template-event-recap");
      await app.find("template-dialog");
      await app.click("template-start");
      const missing = await app.text("template-missing");
      assert.match(missing, /Event name/);
      assert.ok(await app.exists("template-dialog"), "the dialog stays open");
      await app.type("template-field-subject", "Harbour Summit");
      await app.type("template-field-date", "14 March 2026");
      await app.type("template-field-place", "Lisbon");
      await app.click("template-start");
      await app.find("editor");
      const view = await app.waitFor(async () => {
        const p = await app.project();
        return p && p.name.startsWith("Event recap") ? p : null;
      }, "the template project");
      assert.match(view.name, /Harbour Summit/);
      const user = await app.find("msg-user", 30000);
      const brief = await app.textOf(user);
      assert.match(brief, /Make an event recap/);
      assert.match(brief, /Place: Lisbon/);
      // The brief went to the (mock) assistant: wait for its turn to end before leaving.
      await app.find("turn-usage", 60000);
    },
  },

  {
    name: "assistant (mock Messages API): a turn edits the timeline, 'Changed N items', 'Undo this turn', cost shown",
    async run(ctx) {
      const { app } = ctx;
      await newProject(app, "Assistant E2E");
      if (!(await app.exists("chat"))) await app.click("toggle-chat");
      await app.find("chat");
      await app.type("composer", "Add an opening title please");
      await app.click("send");
      const review = await app.find("turn-review", 60000);
      assert.match(await app.textOf(review), /Changed 1 item/);
      let view = await app.project();
      const title = view.items.find((i) => i.kind === "text");
      assert.ok(title, "the assistant added a title");
      assert.equal(title.text, "Welcome");
      await app.waitFor(async () => (await app.count(`[data-testid="clip"][data-id="${title.id}"]`)) === 1, "the title on the timeline");
      const usage = await app.text("turn-usage");
      ctx.log(`    turn: ${usage}`);
      assert.match(usage, /≈\s*\$\d/, "the turn's cost is shown");
      const chats = path.join(view.bundle, "agent", "chats");
      assert.ok(fs.existsSync(chats) && fs.readdirSync(chats).some((f) => f.endsWith(".jsonl")), "the chat is saved in the bundle");

      await app.click("turn-undo");
      await app.waitFor(async () => {
        const p = await app.project();
        return p.items.some((i) => i.kind === "text") ? null : p;
      }, "the turn to be undone");
      await app.waitFor(async () => (await (await app.find("turn-review")).getAttribute("class")).includes("undone"), "the turn marked undone");
      assert.equal(await app.count(`[data-testid="clip"][data-id="${title.id}"]`), 0);
    },
  },

  {
    name: "generate: the estimate shows, confirming asks again, cancel spends nothing",
    async run(ctx) {
      const { app } = ctx;
      await newProject(app, "Generate E2E");
      await app.click("rail-generate");
      await app.find("generate-panel");
      await app.type("gen-prompt", "A lighthouse at dusk, wide shot");
      const estimate = await app.find("gen-estimate", 20000);
      ctx.log(`    estimate: ${await app.textOf(estimate)}`);
      assert.match(await app.textOf(estimate), /\$/);
      const start = await app.find("gen-start");
      assert.equal(await app.textOf(start), "Generate");
      await app.click("gen-start");
      await app.waitFor(async () => /^Confirm: spend/.test(await app.textOf(start)), "the confirmation step");
      await app.clickText("Cancel", "generate-panel");
      await app.waitFor(async () => (await app.textOf(start)) === "Generate", "back to Generate");
      assert.deepEqual(JSON.parse(await app.invoke("generation_jobs")), [], "no job was started");
      assert.equal(await app.count("gen-job"), 0);
    },
  },

  {
    // fix-render-and-edit-correctness task 4.3, app path.
    name: "generate resume: the app is killed while a generation runs; reopened, it resumes with one submission",
    timeoutMs: 8 * 60 * 1000,
    async run(ctx) {
      const { fal } = ctx;
      const project = "Generate Resume E2E";
      await newProject(ctx.app, project);
      await ctx.app.click("rail-generate");
      await ctx.app.find("generate-panel");
      await ctx.app.type("gen-prompt", "A harbour at night, wide shot");
      await ctx.app.find("gen-estimate", 20000);
      const start = await ctx.app.find("gen-start");
      await ctx.app.click("gen-start");
      await ctx.app.waitFor(async () => /^Confirm: spend/.test(await ctx.app.textOf(start)), "the confirmation step");
      await ctx.app.click("gen-start");
      const postsBefore = fal.posts();
      const jobs = async () => JSON.parse(await ctx.app.invoke("generation_jobs"));
      // Running at the provider: submitted once, the handle saved, polling.
      const job = await ctx.app.waitFor(async () => {
        const [j] = await jobs();
        return j && j.status === "running" && j.remote && fal.statusPolls() >= 2 ? j : null;
      }, "the job to run at the stand-in provider", 60000);
      assert.equal(fal.posts(), postsBefore + 1, "one submission");
      assert.ok(job.itemId, "the job has a placeholder");
      ctx.log(`    job ${job.id} running (placeholder ${job.itemId}); killing the app`);

      await ctx.kill();
      fal.finish(); // the provider finishes while the app is gone
      await ctx.relaunch();
      const view = await openFromHome(ctx.app, project);
      assert.ok(view.items.some((i) => i.id === job.itemId), "the placeholder survived the kill");
      // The app gives a worker up after a minute without progress, then resumes its queued request.
      const t0 = Date.now();
      const done = await ctx.app.waitFor(async () => {
        const j = (await jobs()).find((x) => x.id === job.id);
        return j && (j.status === "succeeded" || j.status === "failed") ? j : null;
      }, "the job to resume and finish", 4 * 60 * 1000);
      ctx.log(`    resumed and ${done.status} ${((Date.now() - t0) / 1000).toFixed(0)} s after reopening`);
      assert.equal(done.status, "succeeded", done.error ?? "");
      assert.equal(fal.posts(), postsBefore + 1, "resuming did not submit again");
      const filled = await ctx.app.waitFor(async () => {
        const d = await detail(ctx.app, job.itemId).catch(() => null);
        return d && !d.pending && d.mediaItemId ? d : null;
      }, "the placeholder to be filled");
      ctx.log(`    placeholder filled with media ${filled.mediaItemId}`);
    },
  },

  {
    // unify-agent-tool-layer task 4.2.
    name: "mcp not in front: a session edits project P while the window shows Q; P's history gains it, Q is untouched",
    timeoutMs: 5 * 60 * 1000,
    async run(ctx) {
      const { app } = ctx;
      const [P, Q] = ["Behind P E2E", "Front Q E2E"];
      const addTitle = async () => {
        await app.click("rail-text");
        await app.click("text-preset-headline");
        return app.waitFor(async () => (await app.project()).items.find((i) => i.kind === "text"), "a title");
      };
      await newProject(app, P);
      const title = await addTitle();
      await newProject(app, Q);
      await addTitle();
      const front = await app.project();
      assert.equal(front.name, Q);

      const mcp = startMcp(ctx, ["--project", P, "--turn", "e2e-turn"]);
      try {
        await mcp.request("initialize", {
          protocolVersion: "2025-06-18",
          capabilities: {},
          clientInfo: { name: "e2e-client", version: "1.0" },
        });
        const printed = await mcp.call("print_timeline", {});
        const text = printed.structuredContent?.text ?? "";
        // The title's element from its id to its name attribute (one line, or one attribute per line).
        const lines = text.split("\n");
        const at = lines.findIndex((l) => l.includes(`id="${title.id}"`));
        assert.ok(at >= 0, `P's title is printed: ${text}`);
        const end = lines.findIndex((l, i) => i >= at && /name="[^"]*"/.test(l));
        assert.ok(end >= at && end - at < 20, `P's title has a name: ${text}`);
        const line = lines.slice(at, end + 1).join("\n");
        const renamed = line.replace(/name="[^"]*"/, 'name="Edited by MCP"');
        const before = (await mcp.call("timeline_history", {})).structuredContent.revisions.length;
        const r = await mcp.call("apply_timeline", { rev: printed.structuredContent.rev, edits: [{ oldText: line, newText: renamed }] });
        assert.equal(r.isError, false, JSON.stringify(r));
        const history = (await mcp.call("timeline_history", {})).structuredContent.revisions;
        assert.equal(history.length, before + 1, "P's history gains one revision");
        const last = history[history.length - 1];
        ctx.log(`    P rev ${last.rev}: ${last.source} ${last.summary} (${last.actor?.client ?? "no client"})`);
        assert.equal(last.source, "mcp");
        assert.equal(last.actor?.client, "e2e-client");
      } finally {
        mcp.close();
      }

      // The window still shows Q, unchanged.
      const after = await app.project();
      assert.equal(after.name, Q, "Q is still in front");
      assert.equal(after.rev, front.rev, "Q has no new revision");
      assert.deepEqual(shape(after), shape(front), "Q's items are unchanged");
      // P shows the edit when the person opens it.
      await openFromHome(app, P);
      await app.waitFor(async () => (await detail(app, title.id)).name === "Edited by MCP", "P's title renamed in the app");
    },
  },

  {
    // unify-agent-tool-layer task 6.2 (and 8.2's timings on Windows).
    name: "agent spend: an MCP agent's request shows a price card; declining spends nothing, approving starts the job",
    timeoutMs: 5 * 60 * 1000,
    async run(ctx) {
      const { app, fal } = ctx;
      fal.finish(); // the stand-in completes every request at once
      const project = "Price Card E2E";
      await newProject(app, project);
      const mcp = startMcp(ctx, ["--project", project]);
      try {
        await mcp.request("initialize", {
          protocolVersion: "2025-06-18",
          capabilities: {},
          clientInfo: { name: "e2e-agent", version: "1.0" },
        });
        const args = { prompt: "A lighthouse at dusk (price card e2e)" };
        const quote = async () => {
          const q = await mcp.call("generate_image", args);
          assert.equal(q.isError, false, JSON.stringify(q));
          return q.structuredContent.quoteId;
        };
        const postsBefore = fal.posts();
        const revBefore = (await app.project()).rev;

        // The agent confirms its quote; the person sees the card and declines.
        const declined = mcp.call("generate_image", { ...args, confirm: await quote() });
        const card = await app.find("spend-card", 30000);
        const text = await app.textOf(card);
        ctx.log(`    card: ${text.replace(/\s+/g, " ").slice(0, 160)}`);
        assert.match(text, /e2e-agent/);
        assert.match(text, /\$0\.025/);
        assert.match(text, /price card e2e/);
        await app.click("spend-decline");
        const d = await declined;
        assert.equal(d.structuredContent?.error?.code, "confirm.declined", JSON.stringify(d));
        assert.equal(fal.posts(), postsBefore, "nothing submitted after a decline");
        assert.equal((await app.project()).rev, revBefore, "no placeholder after a decline");

        // Approved: the placeholder appears at once and the worker fills it.
        const approved = mcp.call("generate_image", { ...args, confirm: await quote() });
        await app.find("spend-card", 30000);
        const t0 = Date.now();
        await app.click("spend-approve");
        const r = await approved;
        const tApproved = Date.now() - t0;
        assert.equal(r.isError, false, JSON.stringify(r));
        const { jobId, itemId, line } = r.structuredContent;
        assert.ok(line.includes(itemId) && /pending/.test(line), line);
        const placed = await app.waitFor(async () => {
          const p = await app.project();
          return p.items.find((i) => i.id === itemId) ? p : null;
        }, "the placeholder on the timeline");
        const filled = await app.waitFor(async () => {
          const it = await detail(app, itemId).catch(() => null);
          return it && !it.pending && it.mediaItemId ? it : null;
        }, "the placeholder to be filled", 90000);
        const tFilled = Date.now() - t0;
        ctx.log(`    approve → tool result ${tApproved} ms; → filled ${tFilled} ms (job ${jobId}, media ${filled.mediaItemId}, rev ${placed.rev})`);
        assert.equal(fal.posts(), postsBefore + 1, "one submission");

        // The agent's undo removes placeholder and fill together.
        const u = await mcp.call("undo_my_changes", {});
        assert.equal(u.isError, false, JSON.stringify(u));
        await app.waitFor(async () => !(await app.project()).items.some((i) => i.id === itemId), "the clip to be undone");
      } finally {
        mcp.close();
      }
    },
  },

  {
    // unify-agent-tool-layer task 7.1.
    name: "feedback: an agent's send_feedback appears in Help > Feedback to edit, copy and delete; no network request",
    timeoutMs: 3 * 60 * 1000,
    async run(ctx) {
      const { app, fal } = ctx;
      // Every way out the command line knows (proxies, provider and update addresses) leads to this trap.
      let connections = 0;
      const trap = net.createServer((s) => {
        connections += 1;
        s.destroy();
      });
      await new Promise((r) => trap.listen(0, "127.0.0.1", r));
      const trapUrl = `http://127.0.0.1:${trap.address().port}`;
      const blocked = Object.fromEntries(
        ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "OVE_FAL_QUEUE_URL", "OVE_OPENAI_URL", "OVE_ELEVENLABS_URL", "OVE_ANTHROPIC_BASE_URL", "OVE_UPDATE_URL"].map((k) => [k, trapUrl]),
      );
      const falBefore = fal.posts() + fal.statusPolls();
      const summary = "The apply_timeline receipt does not say which track a moved item ended on (feedback e2e).";
      const mcp = startMcp(ctx, [], blocked);
      try {
        await mcp.request("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e-agent", version: "1.0" } });
        const r = await mcp.call("send_feedback", { category: "confusing", severity: "low", summary });
        assert.equal(r.isError, false, JSON.stringify(r));
        assert.equal(r.structuredContent.stored, true);
        assert.equal(r.structuredContent.sent, false);
        const again = await mcp.call("send_feedback", { category: "confusing", severity: "low", summary });
        assert.equal(again.structuredContent.duplicate, true, "an exact duplicate is not stored again");
      } finally {
        mcp.close();
        trap.close();
      }
      assert.equal(connections, 0, "the MCP session made no network connection");

      await goHome(app);
      await app.click("help");
      await app.click("help-feedback");
      await app.find("feedback-queue");
      const rows = await app.waitFor(async () => {
        const list = (await app.invoke("feedback_drafts"));
        return list.length ? list : null;
      }, "the report in the queue");
      const mine = rows.find((d) => d.summary === summary);
      assert.ok(mine, JSON.stringify(rows));
      assert.equal(mine.from, "e2e-agent");
      const row = await app.find("feedback-draft");
      assert.match(await app.textOf(row), /e2e-agent/);
      // Edit and save it, then delete it.
      await app.type("feedback-draft-text", `${summary} Seen twice.`);
      await app.click("feedback-draft-save");
      await app.waitFor(async () => {
        const d = (await app.invoke("feedback_drafts")).find((x) => x.id === mine.id);
        return d && d.edited && /Seen twice\.$/.test(d.summary);
      }, "the edit to be saved");
      await app.click("feedback-draft-delete");
      await app.waitFor(async () => !(await app.invoke("feedback_drafts")).some((x) => x.id === mine.id), "the report to be deleted");
      await app.clickText("Close", "feedback-queue");
      await app.waitFor(async () => !(await app.exists("feedback-queue")), "the Feedback window to close");
      assert.equal(fal.posts() + fal.statusPolls(), falBefore, "nothing reached a provider");
      ctx.log(`    report ${mine.id} queued, edited and deleted; 0 network connections`);
    },
  },

  {
    name: "settings: every section opens; a rebound shortcut persists; the assistant key test (mock)",
    async run(ctx) {
      const { app } = ctx;
      await goHome(app);
      await app.click("nav-settings");
      await app.find("settings");
      for (const s of ["library", "generation", "assistant", "agents", "shortcuts", "privacy", "about"]) {
        await app.click(`set-${s}`);
        const label = await app.text(`set-${s}`);
        await app.waitFor(async () => {
          const heads = await app.all('[data-testid="settings"] section h3');
          const texts = await Promise.all(heads.map((h) => app.textOf(h)));
          return texts.some((t) => label.startsWith(t) || t.startsWith(label.split(" ")[0]));
        }, `the ${label} section`);
      }
      await app.click("set-shortcuts");
      await app.click("bind-split");
      await app.waitFor(async () => (await app.text("bind-split")) === "Press a key…", "key capture");
      await app.keys("x");
      await app.waitFor(async () => (await app.text("bind-split")).toLowerCase() === "x", "split bound to X");
      // Away and back: still X (stored with the app's web data; the restart flow checks it after a relaunch).
      await app.click("nav-home");
      await app.click("nav-settings");
      await app.click("set-shortcuts");
      assert.equal((await app.text("bind-split")).toLowerCase(), "x");

      await app.click("set-assistant");
      await app.click("agent-key-test");
      await app.waitFor(async () => !/Checking/.test(await app.text("agent-key-check")), "the key check", 30000);
      const check = await app.find("agent-key-check");
      ctx.log(`    key test: ${await app.textOf(check)}`);
      assert.match(await check.getAttribute("class"), /key-ok/, await app.textOf(check));
      ctx.rebound = true;
    },
  },

  {
    name: "resilience: a forced panic in a worker and in the viewer shows an error; the app stays usable; the log has the backtrace",
    async run(ctx) {
      const { app } = ctx;
      await goHome(app);
      await app.takePageEvents();
      // A background worker panics: the command answers with the contained error instead of hanging.
      await assert.rejects(app.invoke("debug_panic_worker"), /worker_panic/, "the worker's panic comes back as worker_panic");
      // The viewer thread panics: the error reaches the window as a toast and the thread keeps serving.
      await app.invoke("debug_panic_viewer");
      // Exactly the viewer's error, as a worker_panic toast; anything else on the page is a problem of this flow.
      const seen = { regionErrors: [], toasts: [], console: [] };
      await app.waitFor(async () => {
        const e = await app.takePageEvents();
        for (const k of Object.keys(seen)) seen[k].push(...e[k]);
        return seen.toasts.length > 0;
      }, "an error toast for the viewer's panic");
      assert.deepEqual(seen.toasts.map((t) => t.code), ["worker_panic"], "one toast, the viewer's worker_panic");
      assert.deepEqual([...seen.regionErrors, ...seen.console], [], "no error cards or console errors from the panics");
      // The window still works: a dialog opens and closes, and a project opens and imports a clip.
      await app.click("nav-changelog");
      await app.find("whats-new");
      await app.keys(Key.ESCAPE);
      await app.waitGone("whats-new");
      await newProject(app, "After Panics");
      await importWithButton(ctx, [ctx.media.video]);
      // Both panics are in the log with a backtrace (the run's log check allows exactly these two messages).
      const records = logRecords(app.info.logDir);
      for (const what of ["forced worker panic (diagnostics)", "forced viewer panic (diagnostics)"]) {
        const r = records.find((x) => x.includes(`panic: ${what}`));
        assert.ok(r, `"${what}" is in the log`);
        assert.match(r, /backtrace:\n\s*\S/, `"${what}" has a backtrace`);
      }
    },
  },

  {
    name: "resilience: close and reopen the app; the project, its edits and settings are intact",
    async run(ctx) {
      let { app } = ctx;
      const { media } = ctx;
      await newProject(app, "Reopen E2E");
      let view = await importWithButton(ctx, [media.video]);
      const [clip] = mainClips(view);
      await clickClip(app, clip.id, 0.5);
      await app.click("split");
      await app.waitFor(async () => mainClips(await app.project()).length === 2, "the split");
      await app.find("inspector");
      await app.setRange("insp-opacity", 0.25);
      await app.waitFor(async () => (await detail(app, clip.id)).opacity === 0.25, "opacity 0.25");
      view = await app.project();
      const before = shape(view);

      await ctx.restart(); // closes the window like the person would, then launches the app again
      app = ctx.app;
      assert.ok(!fs.existsSync(path.join(view.bundle, ".lock.holder")), "the project lock was released on close");
      const after = shape(await openFromHome(app, "Reopen E2E"));
      assert.deepEqual(after.items, before.items, "items survive a restart");
      assert.equal((await detail(app, clip.id)).opacity, 0.25);
      if (ctx.rebound && WINDOWS) {
        // Shortcuts are kept in the web profile, and msedgedriver gives every launch a new, empty one.
        ctx.log("    not checked on Windows: the rebound shortcut after a restart (the driver starts each launch with a new web profile)");
      } else if (ctx.rebound) {
        await goHome(app);
        await app.click("nav-settings");
        await app.click("set-shortcuts");
        assert.equal((await app.text("bind-split")).toLowerCase(), "x", "the rebound shortcut survives a restart");
        await app.click("bind-reset-all");
        await app.waitFor(async () => (await app.text("bind-split")).toLowerCase() === "s", "defaults restored");
      }
    },
  },

  {
    name: "resilience: kill the app mid-session and relaunch; no edit is lost",
    async run(ctx) {
      let { app } = ctx;
      const { media } = ctx;
      await newProject(app, "Crash E2E");
      let view = await importWithButton(ctx, [media.video, media.audio]);
      const [clip] = mainClips(view);
      await clickClip(app, clip.id, 0.4);
      await app.click("split");
      await app.waitFor(async () => mainClips(await app.project()).length === 2, "the split");
      await app.find("inspector");
      await app.click("insp-duplicate");
      await app.waitFor(async () => mainClips(await app.project()).length === 3, "the duplicate");
      view = await app.project();
      const before = shape(view);
      const revs = revisionFiles(view.bundle).length;

      await ctx.kill(); // SIGKILL / TerminateProcess: no shutdown code runs
      assert.ok(fs.existsSync(path.join(view.bundle, ".lock.holder")), "the dead app left its lock behind");
      assert.equal(revisionFiles(view.bundle).length, revs, "every acknowledged edit is on disk");
      await ctx.relaunch();
      app = ctx.app;
      const after = shape(await openFromHome(app, "Crash E2E"));
      assert.deepEqual(after.items, before.items, "items survive a kill");
      assert.equal(after.rev, before.rev, "the same revision");
      // The project is writable again (the stale lock was taken over): an edit lands and is on disk.
      const last = mainClips(await app.project()).at(-1);
      await clickClip(app, last.id, 0.5);
      await app.click("insp-delete");
      await app.waitFor(async () => mainClips(await app.project()).length === 2, "an edit after the relaunch");
      assert.ok(revisionFiles(view.bundle).length > revs, "the edit is a new revision on disk");
      assert.ok(fs.existsSync(path.join(view.bundle, ".lock.holder")), "this app holds the lock now");
    },
  },
  {
    // add-viewer-interaction-and-text-fills groups 1-3 in the built app.
    name: "viewer: overlay lines up with the frame, click selects, a handle drag is one revision, guides, full screen, rates",
    async run(ctx) {
      const { app, media } = ctx;
      await newProject(app, "Viewer E2E");
      const view = await importWithButton(ctx, [media.video]);
      const [clip] = mainClips(view);
      const overlay = await app.find("viewer-overlay");

      // 2.1: the overlay covers the frame within 1 screen pixel (stream backend: the canvas; native: the viewer box).
      const backend = (await app.invoke("app_info")).viewerBackend;
      const frameEl = backend === "stream" ? await app.find("viewer-canvas") : await app.find("viewer-rect");
      const [o, f] = [await app.rect(overlay), await app.rect(frameEl)];
      const off = Math.max(
        Math.abs(o.x - f.x),
        Math.abs(o.y - f.y),
        Math.abs(o.x + o.width - (f.x + f.width)),
        Math.abs(o.y + o.height - (f.y + f.height)),
      );
      ctx.log(`    overlay vs frame (${backend}): ${off.toFixed(2)} px`);
      assert.ok(off <= 1, `overlay lines up with the frame within 1 px (off by ${off})`);

      // 1.3: a click on the picture selects the clip; the handles appear while paused.
      await app.clickAt(overlay, 0, 0);
      await app.waitFor(
        async () => (await (await app.find(`[data-testid="clip"][data-id="${clip.id}"]`)).getAttribute("class")).includes("selected"),
        "the clip to be selected by a click in the viewer",
      );
      await app.find("overlay-handles");

      // 2.2: scale it down with a corner handle, then move it: one revision each, and undo restores.
      // Revisions are counted in the history (revision numbers are not consecutive).
      const changes = async () => (await app.invoke("project_history")).length;
      const rev0 = await changes();
      await app.drag(await app.find("handle-scale-2"), -Math.round(o.width / 4), -Math.round(o.height / 4));
      await app.waitFor(async () => (await changes()) === rev0 + 1, "one revision for the scale drag");
      const scaled = (await detail(app, clip.id)).transform;
      assert.ok(scaled.scale < 0.8, `scaled down (${JSON.stringify(scaled)})`);
      const started = Date.now();
      await app.drag(overlay, Math.round(o.width / 8), 0);
      await app.waitFor(async () => (await changes()) === rev0 + 2, "one revision for the move");
      ctx.log(`    move drag to revision: ${Date.now() - started} ms (WebDriver pointer actions, 8 steps)`);
      const movedT = (await detail(app, clip.id)).transform;
      assert.ok(movedT.x > 50, `moved right (${JSON.stringify(movedT)})`);
      await app.click("undo");
      await app.waitFor(async () => ((await detail(app, clip.id)).transform?.x ?? 0) === (scaled.x ?? 0), "undo restores the position");

      const afterUndo = await changes();
      // Drag preview latency: a transient preview to the next presented frame.
      const presented = async () => (await app.invoke("diagnostics")).viewer.presentedFrames;
      const lat = [];
      for (const x of [100, -100, 200, -200, 0]) {
        const before = await presented();
        const t0 = Date.now();
        await app.invoke("viewer_preview", { preview: { itemId: clip.id, setJson: JSON.stringify({ transform: { ...scaled, x } }) } });
        await app.waitFor(async () => (await presented()) > before, "a preview frame", 10000, 5);
        lat.push(Date.now() - t0);
      }
      await app.invoke("viewer_preview", { preview: null });
      lat.sort((a, b) => a - b);
      ctx.log(`    drag preview to frame: median ${lat[2]} ms, max ${lat[4]} ms`);
      assert.equal(await changes(), afterUndo, "previews make no revision");

      // 3.1: guides are view state only.
      await app.click("guides-button");
      await app.click("guides-title");
      await app.select("guides-mask", "9:16");
      await app.find("guide-layer");
      await app.keys(Key.ESCAPE);
      assert.equal(await changes(), afterUndo, "guides change nothing in the project");

      // 3.3: full screen and back with Esc.
      await app.click("viewer-fullscreen");
      await app.waitFor(async () => (await (await app.find("viewer-area")).getAttribute("data-full")) === "true", "full screen");
      await app.find("fullscreen-controls");
      await sleep(500);
      const full = await app.rect(await app.find("viewer-area"));
      const win = await app.driver.executeScript("return { w: window.innerWidth, h: window.innerHeight };");
      assert.ok(full.width >= win.w - 1 && full.height >= win.h - 1, `the viewer fills the window (${JSON.stringify(full)} vs ${JSON.stringify(win)})`);
      await app.keys(Key.ESCAPE);
      await app.waitFor(async () => (await (await app.find("viewer-area")).getAttribute("data-full")) === "false", "back from full screen");
      await app.find("timeline");

      // 3.4: the rate menu sets 1.5x; 7x is refused with the supported rates.
      await app.click("rate-button");
      await app.click(`[data-testid="rate-item-1.5"]`);
      await app.waitFor(async () => (await app.invoke("playback_state")).rate === 1.5, "rate 1.5");
      // The 1.5x timing itself is measured on the simulated clock (crates/ove-playback/tests/player.rs): a debug build
      // composites far below real time, so wall-clock timing here would measure the compositor, not the rate.
      assert.equal((await app.invoke("playback_state")).rate, 1.5);
      await assert.rejects(
        app.invoke("transport", { command: { kind: "setRate", rate: 7 } }),
        /supported rates: 0.5, 1, 1.25, 1.5, 2/,
      );
      await app.click("rate-button");
      await app.click("rate-item-1");
    },
  },
];
