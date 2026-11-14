// Layout checks against the UI served by Vite with the bridge mocked (no native core).
// Verifies what the native viewer depends on: the rectangle reported over the bridge matches the
// viewer element after every layout change, hiding the viewer reports `visible: false`, and menus
// opened over the viewer are the topmost element. Saves screenshots for visual comparison.
//
// Needs Playwright with a Chromium build available to Node (`require("playwright")`).
// Usage: pnpm e2e   (OVE_SCREENSHOT_DIR=<dir> to choose where screenshots go)
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const here = path.dirname(fileURLToPath(import.meta.url));
const appDir = path.resolve(here, "../..");
const outDir = process.env.OVE_SCREENSHOT_DIR ?? path.join(here, "out");
mkdirSync(outDir, { recursive: true });
const PORT = 5179;
const URL = `http://localhost:${PORT}/`;

let failures = 0;
function check(name, ok, detail = "") {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
  if (!ok) failures++;
}

async function startVite() {
  const proc = spawn("pnpm", ["exec", "vite", "--port", String(PORT), "--strictPort"], {
    cwd: appDir,
    stdio: ["ignore", "pipe", "pipe"],
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("vite did not start")), 30000);
    proc.stdout.on("data", (d) => {
      if (String(d).includes("Local:")) {
        clearTimeout(timer);
        resolve();
      }
    });
    proc.on("exit", (code) => reject(new Error(`vite exited ${code}`)));
  });
  return proc;
}

const frames = (page, n = 3) =>
  page.evaluate((n) => new Promise((r) => {
    let i = 0;
    const tick = () => (++i >= n ? r() : requestAnimationFrame(tick));
    requestAnimationFrame(tick);
  }), n);

async function rectMatches(page, label) {
  await frames(page);
  const { reported, actual } = await page.evaluate(() => {
    const el = document.querySelector('[data-testid="viewer-rect"]');
    const r = el?.getBoundingClientRect();
    return {
      reported: window.__oveMock.lastRect,
      actual: r ? { x: r.left, y: r.top, width: r.width, height: r.height, scaleFactor: devicePixelRatio } : null,
    };
  });
  const ok =
    reported &&
    actual &&
    reported.visible &&
    ["x", "y", "width", "height", "scaleFactor"].every((k) => Math.abs(reported[k] - actual[k]) < 0.5);
  check(`viewer rect reported = element rect (${label})`, !!ok, JSON.stringify({ reported, actual }));
  return reported;
}

async function topmostInside(page, selector, label) {
  const result = await page.evaluate((selector) => {
    const menu = document.querySelector(selector);
    const viewer = document.querySelector('[data-testid="viewer-rect"]')?.getBoundingClientRect();
    if (!menu || !viewer) return { ok: false, why: "missing element" };
    const items = [...menu.querySelectorAll('[role="menuitem"]')];
    const overlapping = items.filter((it) => {
      const r = it.getBoundingClientRect();
      return r.left < viewer.right && r.right > viewer.left && r.top < viewer.bottom && r.bottom > viewer.top;
    });
    const allTop = items.every((it) => {
      const r = it.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return hit && it.contains(hit);
    });
    return { ok: overlapping.length > 0 && allTop, overlapping: overlapping.length, items: items.length };
  }, selector);
  check(`${label} overlaps the viewer and is topmost`, result.ok, JSON.stringify(result));
}

/**
 * Clicks each visible, enabled button inside `scope` (starting from `reset()` each time) and returns the labels of
 * those that changed nothing: no DOM change and no bridge call. The current page, tab or option is skipped.
 */
async function deadControls(page, scope, reset) {
  const sel = `${scope} button:not([disabled]):not([aria-current="page"]):not([aria-selected="true"]):not(.on)`;
  const count = await page.locator(sel).count();
  const dead = [];
  for (let i = 0; i < count; i++) {
    await reset();
    const el = page.locator(sel).nth(i);
    if (!(await el.isVisible())) continue;
    const label = ((await el.getAttribute("aria-label")) ?? (await el.textContent()) ?? "").trim();
    await page.evaluate(() => {
      window.__mutations = 0;
      new MutationObserver((m) => (window.__mutations += m.length)).observe(document.body, {
        subtree: true,
        childList: true,
        attributes: true,
        characterData: true,
      });
      window.__callsBefore = window.__oveMock.calls.length;
    });
    await el.click();
    await frames(page, 4);
    const changed = await page.evaluate(
      () =>
        window.__mutations > 0 ||
        window.__oveMock.calls.slice(window.__callsBefore).some((c) => !c.startsWith("reportUiState")),
    );
    if (!changed) dead.push(label || `button ${i}`);
  }
  await reset();
  return dead;
}

const vite = await startVite();
const browser = await chromium.launch();
try {
  // Screenshots at 2× for people to compare with the reference layout.
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 805 }, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => check("no page errors", false, String(e)));
  await page.goto(URL);
  await page.getByTestId("home").waitFor();
  await page.screenshot({ path: path.join(outDir, "home.png") });
  check("home renders", true);

  // No dead controls on Home: every visible, enabled button does something when clicked.
  const dead = await deadControls(page, '[data-testid="home"]', async () => {
    await page.goto(URL);
    await page.getByTestId("home").waitFor();
  });
  check("every Home control responds", dead.length === 0, dead.join(", "));

  // Changelog and Send feedback.
  await page.getByTestId("nav-changelog").click();
  await page.getByTestId("whats-new").waitFor();
  await page.screenshot({ path: path.join(outDir, "home-whats-new.png") });
  check("what's new lists the known limitations", (await page.getByTestId("whats-new-limitations").textContent()).includes("540p"));
  await page.keyboard.press("Escape");
  await page.getByTestId("nav-feedback").click();
  await page.getByTestId("feedback-text").fill("The export stopped.");
  await page.getByTestId("feedback-env").getByText("startup: app", { exact: false }).waitFor();
  await page.getByText("What will be copied").click();
  await page.screenshot({ path: path.join(outDir, "home-feedback.png") });
  check("feedback report has the environment", (await page.getByTestId("feedback-preview").textContent()).includes("--- Environment ---"));
  await page.keyboard.press("Escape");
  check("feedback dialog closes", (await page.getByTestId("feedback-dialog").count()) === 0);

  await page.getByTestId("home-open").click();
  await page.getByTestId("editor").waitFor();
  await frames(page, 5);
  await page.screenshot({ path: path.join(outDir, "editor.png") });
  const first = await rectMatches(page, "initial");
  check("scale factor reported", first?.scaleFactor === 2, String(first?.scaleFactor));

  // Transport via keyboard updates the timecode.
  for (let i = 0; i < 10; i++) await page.keyboard.press("ArrowRight");
  await frames(page);
  check("10 × step = frame 10", (await page.getByTestId("timecode").textContent()) === "00:00:00:10");

  // Window resize.
  await page.setViewportSize({ width: 1180, height: 720 });
  await rectMatches(page, "after window resize");

  // Panel collapse moves the viewer without necessarily resizing it.
  const before = await page.evaluate(() => window.__oveMock.lastRect.x);
  await page.getByTestId("collapse-panel").click();
  const afterCollapse = await rectMatches(page, "after side panel collapse");
  check("viewer moved left when the panel collapsed", afterCollapse.x < before, `${before} → ${afterCollapse.x}`);
  await page.setViewportSize({ width: 1440, height: 805 });
  await frames(page, 4);
  await page.screenshot({ path: path.join(outDir, "editor-panel-collapsed.png") });
  await page.getByTestId("rail-media").click();

  await page.getByTestId("toggle-timeline").click();
  await rectMatches(page, "timeline hidden");
  await page.screenshot({ path: path.join(outDir, "editor-timeline-hidden.png") });
  await page.getByTestId("toggle-timeline").click();

  await page.getByTestId("close-chat").click();
  await rectMatches(page, "chat hidden");
  await page.screenshot({ path: path.join(outDir, "editor-chat-hidden.png") });
  await page.getByTestId("toggle-chat").click();
  await rectMatches(page, "chat shown again");

  // Collapsing the viewer area hides the surface.
  await page.getByTestId("toggle-viewer").click();
  await frames(page);
  const hidden = await page.evaluate(() => window.__oveMock.lastRect);
  check("collapsed viewer reports visible=false", hidden && hidden.visible === false, JSON.stringify(hidden));
  check("viewer element removed", (await page.getByTestId("viewer-area").count()) === 0);
  await page.getByTestId("toggle-viewer").click();
  await rectMatches(page, "viewer restored");

  // Menus over the viewer.
  await page.getByTestId("viewer-zoom-button").click();
  await topmostInside(page, '[data-testid="viewer-zoom-menu"]', "zoom dropdown");
  await page.screenshot({ path: path.join(outDir, "editor-menu-over-viewer.png") });
  await page.keyboard.press("Escape");
  const box = await page.getByTestId("viewer-rect").boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { button: "right" });
  await topmostInside(page, '[data-testid="context-menu"]', "context menu");
  await page.screenshot({ path: path.join(outDir, "editor-context-menu.png") });
  await page.keyboard.press("Escape");

  // Diagnostics overlay toggles with Ctrl+Shift+D.
  await page.keyboard.press("Control+Shift+D");
  check("diagnostics overlay opens", await page.getByTestId("diagnostics").isVisible());
  await page.screenshot({ path: path.join(outDir, "editor-diagnostics.png") });

  // Timeline exchange: export tab and import button.
  await page.keyboard.press("Control+Shift+D");
  await page.getByTestId("open-export").click();
  await page.getByTestId("export-kind-timeline").click();
  await page.getByTestId("timeline-formats").getByText("XML", { exact: true }).click();
  await page.screenshot({ path: path.join(outDir, "export-timeline.png") });
  await page.getByTestId("timeline-export-start").click();
  // When something won't carry over, the dialog lists it before writing.
  await page.locator("[data-testid=timeline-export-done], [data-testid=timeline-report]").first().waitFor();
  if (await page.getByTestId("timeline-report").isVisible()) {
    await page.screenshot({ path: path.join(outDir, "export-timeline-report.png") });
    await page.getByTestId("timeline-export-continue").click();
  }
  await page.getByTestId("timeline-export-done").waitFor();
  check("timeline export saved", (await page.getByTestId("timeline-export-done").textContent()).includes(".xml"));
  await page.getByText("Done", { exact: true }).click();
  await page.getByTestId("import-timeline").click();
  await page.getByTestId("toast").first().waitFor();
  check("timeline import reported", (await page.getByTestId("toast").first().textContent()).includes("Imported 1 clip"));
  await page.screenshot({ path: path.join(outDir, "import-timeline.png") });

  // Scale factor 1 display.
  const ctx1 = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
  const p1 = await ctx1.newPage();
  await p1.goto(URL);
  await p1.getByTestId("home-open").click();
  const r1 = await rectMatches(p1, "1× display");
  check("scale factor 1 reported", r1?.scaleFactor === 1);
  await ctx1.close();
  await ctx.close();
} finally {
  await browser.close();
  vite.kill();
}
console.log(`screenshots: ${outDir}`);
if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
