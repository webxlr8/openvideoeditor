// Runs before every composition, in a fresh sandbox per frame. Defines the JSX factory, the animation helpers,
// and makes time and randomness deterministic. Nothing here touches the host: there is no host API to touch.
"use strict";
(() => {
  const G = globalThis;

  // ---- determinism -------------------------------------------------------------------------------------------
  const RealDate = Date;
  function FixedDate(...args) {
    if (!new.target) return new RealDate(0).toISOString();
    return args.length ? new RealDate(...args) : new RealDate(0);
  }
  FixedDate.prototype = RealDate.prototype;
  FixedDate.now = () => 0;
  FixedDate.UTC = RealDate.UTC;
  FixedDate.parse = RealDate.parse;
  G.Date = FixedDate;

  // Small counter-based generator; the sequence restarts every frame, so frames are reproducible.
  const gen = (s) => () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  Math.random = gen(0x5eed);
  const hash = (str) => {
    let x = 2166136261;
    for (let i = 0; i < str.length; i++) x = Math.imul(x ^ str.charCodeAt(i), 16777619);
    return x | 0;
  };
  /** Deterministic number in [0, 1) for a key (number or string): the same key gives the same value on every frame. */
  G.random = (key) => gen(typeof key === "number" ? key * 7919 : hash(String(key)))();

  // ---- console (captured, shown as diagnostics by `check`) ---------------------------------------------------
  const logs = [];
  const log = (level) => (...a) => {
    if (logs.length < 50) logs.push(level + ": " + a.map((v) => (typeof v === "string" ? v : JSON.stringify(v))).join(" "));
  };
  G.console = { log: log("log"), info: log("info"), warn: log("warn"), error: log("error"), debug: log("debug") };

  // ---- JSX ---------------------------------------------------------------------------------------------------
  const ELEMENT = Symbol("ove.element");
  const Fragment = Symbol("ove.fragment");
  function h(type, props, ...children) {
    return { [ELEMENT]: true, type, props: props || {}, children };
  }
  G.h = h;
  G.Fragment = Fragment;
  G.React = { createElement: h, Fragment };

  /** A full-frame flex column, the usual root of a composition. */
  G.Fill = ({ style, children, ...rest }) =>
    h("div", { ...rest, style: { position: "absolute", left: 0, top: 0, right: 0, bottom: 0, display: "flex", flexDirection: "column", ...style } }, children);

  // ---- animation helpers -------------------------------------------------------------------------------------
  const clamp = (v, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, v));
  G.clamp = clamp;
  G.mix = (a, b, t) => a + (b - a) * t;

  const bezier = (x1, y1, x2, y2) => {
    const a = (p1, p2) => 1 - 3 * p2 + 3 * p1, b = (p1, p2) => 3 * p2 - 6 * p1, c = (p1) => 3 * p1;
    const at = (t, p1, p2) => ((a(p1, p2) * t + b(p1, p2)) * t + c(p1)) * t;
    const slope = (t, p1, p2) => 3 * a(p1, p2) * t * t + 2 * b(p1, p2) * t + c(p1);
    return (x) => {
      if (x <= 0) return 0;
      if (x >= 1) return 1;
      let t = x;
      for (let i = 0; i < 8; i++) {
        const d = slope(t, x1, x2);
        if (Math.abs(d) < 1e-6) break;
        t -= (at(t, x1, x2) - x) / d;
      }
      return at(clamp(t), y1, y2);
    };
  };
  G.easing = {
    linear: (t) => t,
    in: (t) => t * t * t,
    out: (t) => 1 - Math.pow(1 - t, 3),
    inOut: (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2),
    back: (t) => 1 + 2.7 * Math.pow(t - 1, 3) + 1.7 * Math.pow(t - 1, 2),
    elastic: (t) => (t <= 0 ? 0 : t >= 1 ? 1 : Math.pow(2, -10 * t) * Math.sin((t * 10 - 0.75) * ((2 * Math.PI) / 3)) + 1),
    bounce: (t) => {
      const n = 7.5625, d = 2.75;
      if (t < 1 / d) return n * t * t;
      if (t < 2 / d) return n * (t -= 1.5 / d) * t + 0.75;
      if (t < 2.5 / d) return n * (t -= 2.25 / d) * t + 0.9375;
      return n * (t -= 2.625 / d) * t + 0.984375;
    },
    cubicBezier: bezier,
  };

  /**
   * Maps `value` through the piecewise-linear curve `input` → `output` (equal-length ascending arrays).
   * Options: `easing` (applied per segment), `clamp` (default true; false extrapolates).
   */
  G.interpolate = (value, input, output, options = {}) => {
    if (!Array.isArray(input) || !Array.isArray(output) || input.length !== output.length || input.length < 2)
      throw new RangeError("interpolate needs two arrays of the same length (at least 2)");
    const ease = options.easing || ((t) => t);
    const doClamp = options.clamp !== false;
    let i = 1;
    while (i < input.length - 1 && value > input[i]) i++;
    const a = input[i - 1], b = input[i];
    let t = b === a ? 1 : (value - a) / (b - a);
    if (doClamp) t = clamp(t);
    return output[i - 1] + (output[i] - output[i - 1]) * ease(t);
  };

  /** Damped spring from `from` to `to` starting at `delay` seconds (closed form, so any frame renders alone). */
  G.spring = ({ time, delay = 0, from = 0, to = 1, stiffness = 170, damping = 20, mass = 1 } = {}) => {
    const t = Math.max(0, (time ?? 0) - delay);
    const w0 = Math.sqrt(stiffness / mass);
    const zeta = damping / (2 * Math.sqrt(stiffness * mass));
    let x;
    if (zeta < 1) {
      const wd = w0 * Math.sqrt(1 - zeta * zeta);
      x = Math.exp(-zeta * w0 * t) * (Math.cos(wd * t) + ((zeta * w0) / wd) * Math.sin(wd * t));
    } else {
      x = Math.exp(-w0 * t) * (1 + w0 * t);
    }
    return to + (from - to) * x;
  };

  /** Local time for the `index`-th of several staggered elements: `time - delay - index * step`. */
  G.stagger = (time, index, step = 0.08, delay = 0) => time - delay - index * step;

  // ---- element tree → plain data -----------------------------------------------------------------------------
  const KEEP = new Set(["style", "tw", "src", "width", "height", "id", "className", "alt"]);
  function plain(v) {
    if (v === null || typeof v === "string" || typeof v === "boolean") return v;
    if (typeof v === "number") return Number.isFinite(v) ? v : 0;
    if (Array.isArray(v)) return v.map(plain);
    if (typeof v === "object") {
      const o = {};
      for (const k of Object.keys(v)) {
        const x = v[k];
        if (typeof x !== "function" && typeof x !== "symbol" && x !== undefined) o[k] = plain(x);
      }
      return o;
    }
    return null;
  }
  let nodes = 0;
  function resolve(v, depth, out) {
    if (depth > 200) throw new RangeError("element tree is nested too deeply");
    if (++nodes > 20000) throw new RangeError("element tree has too many elements (limit 20000)");
    if (v === null || v === undefined || v === false || v === true) return;
    if (typeof v === "string" || typeof v === "number") return void out.push(String(v));
    if (Array.isArray(v)) {
      for (const c of v) resolve(c, depth + 1, out);
      return;
    }
    if (typeof v === "object" && v[ELEMENT]) {
      const { type, props, children } = v;
      const kids = children.length ? children : props.children !== undefined ? [props.children] : [];
      if (type === Fragment) return resolve(kids, depth + 1, out);
      if (typeof type === "function") {
        const p = { ...props };
        if (children.length) p.children = children.length === 1 ? children[0] : children;
        return resolve(type(p), depth + 1, out);
      }
      if (typeof type === "string") {
        const keep = {};
        const dropped = [];
        for (const k of Object.keys(props)) {
          if (k === "children" || k === "key") continue;
          if (KEEP.has(k)) keep[k] = plain(props[k]);
          else if (typeof props[k] !== "function") dropped.push(k);
        }
        const c = [];
        resolve(kids, depth + 1, c);
        return void out.push({ tag: type, props: keep, children: c, dropped });
      }
      throw new TypeError("element type must be a tag name or a component function, got " + typeof type);
    }
    throw new TypeError("cannot render a value of type " + typeof v + " (return JSX, a string or null)");
  }
  G.__ove_tree = (value) => {
    nodes = 0;
    const out = [];
    resolve(value, 0, out);
    return JSON.stringify({ nodes: out, logs });
  };
})();
