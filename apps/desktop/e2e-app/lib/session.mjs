// One WebDriver session = one launch of the real app (tauri-driver starts it through WebKitWebDriver on Linux and
// msedgedriver on Windows). Helpers address the UI by `data-testid` and read the native side through the same
// `invoke` bridge the UI uses, so assertions check real state, not just that a click happened.
import fs from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { Builder, By, Key, until } = require("selenium-webdriver");

export { By, Key };

/** `[data-testid="x"]`, or the selector as given when it already looks like CSS. */
export const tid = (id) => (/[[\].#> ]/.test(id) ? id : `[data-testid="${id}"]`);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export { sleep };

/** Records error cards, error toasts and console errors in the page, for the runner to check after each flow. */
const PAGE_HOOKS = `
if (!window.__e2e) {
  window.__e2e = { regionErrors: [], toasts: [], console: [] };
  const seen = new WeakSet();
  const scan = () => {
    for (const el of document.querySelectorAll('[data-testid="region-error"]')) {
      if (seen.has(el)) continue;
      seen.add(el);
      window.__e2e.regionErrors.push((el.dataset.region || "?") + ": " + el.innerText.replace(/\\s+/g, " "));
    }
    for (const el of document.querySelectorAll('[data-testid="toast"]')) {
      if (seen.has(el)) continue;
      seen.add(el);
      window.__e2e.toasts.push({ code: el.dataset.code, text: el.innerText.replace(/\\s+/g, " ").replace(/×$/, "") });
    }
  };
  new MutationObserver(scan).observe(document.documentElement, { subtree: true, childList: true });
  scan();
  const orig = console.error.bind(console);
  console.error = (...a) => {
    window.__e2e.console.push(a.map((x) => (x instanceof Error ? x.stack || x.message : String(x))).join(" "));
    orig(...a);
  };
  window.addEventListener("error", (e) => window.__e2e.console.push("uncaught: " + e.message));
  window.addEventListener("unhandledrejection", (e) =>
    window.__e2e.console.push("unhandled rejection: " + (e.reason && e.reason.message ? e.reason.message : JSON.stringify(e.reason))),
  );
}
`;

export class AppSession {
  /** @param {{server: string, application: string, log: (s: string) => void}} opts */
  constructor(opts) {
    this.opts = opts;
    this.driver = null;
  }

  async start() {
    const caps = {
      browserName: "wry",
      "tauri:options": { application: this.opts.application },
    };
    this.driver = await new Builder().usingServer(this.opts.server).withCapabilities(caps).build();
    await this.driver.manage().setTimeouts({ script: 60000 });
    await this.waitFor(
      async () => (await this.count("home")) + (await this.count("editor")) > 0,
      "the app window to show Home or the editor (a plain `cargo build` of the app loads the Vite dev server " +
        "instead of the built UI: build it with `pnpm tauri build --debug --no-bundle`)",
      60000,
    );
    await this.driver.executeScript(PAGE_HOOKS);
    this.info = await this.invoke("app_info");
    return this;
  }

  /** Ends the session: the window closes and the app quits as when the person closes it. */
  async quit() {
    const d = this.driver;
    this.driver = null;
    if (!d) return;
    await Promise.race([d.quit().catch(() => {}), sleep(20000)]);
  }

  /** Forgets a session whose app was killed (the driver can no longer reach it). */
  async abandon() {
    const d = this.driver;
    this.driver = null;
    if (d) await Promise.race([d.quit().catch(() => {}), sleep(5000)]);
  }

  get alive() {
    return this.driver !== null;
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Finding and acting

  async find(id, timeout = 15000) {
    const by = By.css(tid(id));
    const el = await this.driver.wait(until.elementLocated(by), timeout, `waiting for ${id}`);
    await this.driver.wait(until.elementIsVisible(el), timeout, `waiting for ${id} to be visible`);
    return el;
  }

  async all(id) {
    return this.driver.findElements(By.css(tid(id)));
  }

  async count(id) {
    return (await this.all(id)).length;
  }

  async exists(id) {
    return (await this.count(id)) > 0;
  }

  async click(id, timeout) {
    const el = typeof id === "string" ? await this.find(id, timeout) : id;
    await this.driver.wait(until.elementIsEnabled(el), timeout ?? 15000, `waiting for ${id} to be enabled`);
    await this.scrollIntoView(el);
    await el.click();
    return el;
  }

  /** Clicks the button inside `scope` whose text is `text`. */
  async clickText(text, scope = "body") {
    const els = await this.driver.findElements(By.css(`${tid(scope)} button`));
    for (const el of els) {
      if ((await this.textOf(el)) === text && (await el.isDisplayed())) {
        await this.scrollIntoView(el);
        await el.click();
        return el;
      }
    }
    throw new Error(`no button "${text}" in ${scope}`);
  }

  async text(id) {
    return this.textOf(await this.find(id));
  }

  /** The element's rendered text (`innerText`; WebKitWebDriver's Get Element Text leaves out some inline elements). */
  async textOf(el) {
    return this.driver.executeScript("return arguments[0].innerText.trim()", el);
  }

  async type(id, text, { clear = true } = {}) {
    const el = await this.find(id);
    await this.scrollIntoView(el);
    await el.click();
    if (clear) await this.replaceText(el, text);
    else await el.sendKeys(text);
    return el;
  }

  /** Selects all text in a field and types over it. Separate calls: sent as one sequence after the chord, the
   * edit did not take with WebKitWebDriver. */
  async replaceText(el, text) {
    await el.sendKeys(Key.chord(Key.CONTROL, "a"));
    await el.sendKeys(Key.BACK_SPACE);
    if (text) await el.sendKeys(text);
  }

  async keys(...k) {
    await this.driver.actions({ async: true }).sendKeys(...k).perform();
  }

  async scrollIntoView(el) {
    await this.driver.executeScript("arguments[0].scrollIntoView({block: 'center', inline: 'center'})", el);
  }

  /** Drags `el` by (dx, dy) pixels with real pointer input (down, several moves, up). */
  async drag(el, dx, dy = 0, from = { x: 0, y: 0 }) {
    await this.scrollIntoView(el);
    // Viewport coordinates: the element follows the pointer while dragged, so moves relative to it would compound.
    const r = await this.rect(el);
    const x0 = Math.round(r.x + r.width / 2 + from.x);
    const y0 = Math.round(r.y + r.height / 2 + from.y);
    let chain = this.driver.actions({ async: true }).move({ origin: "viewport", x: x0, y: y0 }).press();
    const steps = 8;
    for (let i = 1; i <= steps; i++) {
      chain = chain.move({
        origin: "viewport",
        x: x0 + Math.round((dx * i) / steps),
        y: y0 + Math.round((dy * i) / steps),
        duration: 25,
      });
    }
    await chain.release().perform();
  }

  /** The element's rectangle in viewport pixels. */
  async rect(el) {
    return this.driver.executeScript(
      "const r = arguments[0].getBoundingClientRect(); return { x: r.left, y: r.top, width: r.width, height: r.height };",
      el,
    );
  }

  /** Clicks at an offset (pixels) from the element's centre. */
  async clickAt(el, x = 0, y = 0) {
    await this.scrollIntoView(el);
    const r = await this.rect(el);
    await this.driver
      .actions({ async: true })
      .move({ origin: "viewport", x: Math.round(r.x + r.width / 2 + x), y: Math.round(r.y + r.height / 2 + y) })
      .click()
      .perform();
  }

  /** Sets a range input the way the person's drag would end: input events, then the release that commits. */
  async setRange(id, value) {
    const el = await this.find(id);
    await this.scrollIntoView(el);
    await this.driver.executeScript(
      `const el = arguments[0];
       const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
       el.focus();
       set.call(el, String(arguments[1]));
       el.dispatchEvent(new Event("input", { bubbles: true }));
       el.dispatchEvent(new Event("change", { bubbles: true }));`,
      el,
      value,
    );
    // React re-renders with the live value; the key release commits (as after arrow keys).
    await sleep(100);
    await this.driver.executeScript(
      `arguments[0].dispatchEvent(new KeyboardEvent("keyup", { bubbles: true, key: "ArrowRight" }));`,
      el,
    );
  }

  /** Picks an option of a <select> by value. */
  async select(id, value) {
    const el = await this.find(id);
    await this.driver.executeScript(
      `const el = arguments[0];
       const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set;
       set.call(el, arguments[1]);
       el.dispatchEvent(new Event("change", { bubbles: true }));`,
      el,
      value,
    );
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Waiting

  async waitFor(fn, what, timeout = 15000, interval = 150) {
    const end = Date.now() + timeout;
    let last;
    for (;;) {
      try {
        const v = await fn();
        if (v) return v;
      } catch (e) {
        last = e;
      }
      if (Date.now() > end) {
        throw new Error(`timed out after ${timeout} ms waiting for ${what}${last ? ` (last error: ${last.message})` : ""}`);
      }
      await sleep(interval);
    }
  }

  async waitGone(id, timeout = 15000) {
    await this.waitFor(async () => !(await this.exists(id)), `${id} to go away`, timeout);
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Native state, through the same bridge the UI uses

  /** Calls a native command; resolves with its data or rejects with its AppError message. */
  async invoke(cmd, args = {}) {
    const r = await this.driver.executeAsyncScript(
      `const cb = arguments[arguments.length - 1];
       window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(
         (data) => cb({ ok: true, data }),
         (error) => cb({ ok: false, error }),
       );`,
      cmd,
      args,
    );
    if (!r.ok) throw new Error(`${cmd}: ${JSON.stringify(r.error)}`);
    return r.data;
  }

  /** The open project as the editor sees it (`null` on Home). */
  async project() {
    return this.invoke("project_view").catch(() => null);
  }

  /** Emits an app event to every listener (the window-drop route uses `open-paths-requested`). */
  async emit(event, payload) {
    await this.invoke("plugin:event|emit", { event, payload });
  }

  /** What the page recorded since the last call (error cards, toasts, console errors). */
  async takePageEvents() {
    return this.driver.executeScript(
      `const e = window.__e2e || { regionErrors: [], toasts: [], console: [] };
       const out = { regionErrors: e.regionErrors.splice(0), toasts: e.toasts.splice(0), console: e.console.splice(0) };
       return out;`,
    );
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Artefacts

  async saveArtefacts(dir, name) {
    fs.mkdirSync(dir, { recursive: true });
    const saved = [];
    if (!this.driver) return saved;
    try {
      const png = await this.driver.takeScreenshot();
      const p = `${dir}/${name}.png`;
      fs.writeFileSync(p, Buffer.from(png, "base64"));
      saved.push(p);
    } catch (e) {
      this.opts.log(`  (no screenshot: ${e.message})`);
    }
    try {
      const html = await this.driver.getPageSource();
      const p = `${dir}/${name}.html`;
      fs.writeFileSync(p, html);
      saved.push(p);
    } catch (e) {
      this.opts.log(`  (no page source: ${e.message})`);
    }
    return saved;
  }
}
