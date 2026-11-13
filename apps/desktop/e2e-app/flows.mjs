// The user flows. Each flow starts on Home in a running app, makes its own project, and checks real outcomes: the
// project the native side holds (through the UI's own bridge), files in the bundle, files written by exports.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

import { Key } from "./lib/session.mjs";
import { WINDOWS, bundleItems, ffprobe, makeSpeech, revisionFiles } from "./lib/support.mjs";

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

// ---------------------------------------------------------------------------------------------------------------
// Flows

export const FLOWS = [
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
        return nb && nb.end < b.end - 0.1 ? p : null;
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
      // The viewer shows the composited timeline at the cut: the viewer presents a new frame (and, where frames are
      // streamed into the page, the canvas shows a real picture, not black), and no error card.
      const presented = async () => (await app.invoke("diagnostics")).viewer.presentedFrames;
      const before = await presented();
      await clickClip(app, first.id, 0.9);
      await app.waitFor(async () => (await presented()) > before, "the viewer to present a frame", 30000);
      if (app.info.viewerBackend === "stream") {
        await app.waitFor(async () => (await litViewerPixels(app)) > 20, "a rendered frame in the viewer canvas", 30000);
      } else ctx.log(`    native viewer (${app.info.viewerBackend}): checked through the presented-frame count`);
      assert.equal(await app.count("region-error"), 0);
    },
  },

  {
    name: "export: MP4 via the Export dialog; FCPXML with the 'what won't carry over' step",
    timeoutMs: 8 * 60 * 1000,
    async run(ctx) {
      const { app, media } = ctx;
      await newProject(app, "Export E2E");
      await importWithButton(ctx, [media.video]);
      await app.click("open-export");
      await app.find("export-dialog");
      await app.clickText("720p", "export-dialog");
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
];
