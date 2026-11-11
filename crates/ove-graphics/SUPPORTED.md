# Motion-graphics code items: the supported subset

A `code` item draws a **composition**: a TSX module whose default export is a pure function of time that returns
JSX. It is rendered on the CPU for every frame, in preview and export alike.

```tsx
export default function Title({ time, duration, width, height, props, assets }) {
  const p = easing.out(clamp(time / 0.6));
  return (
    <Fill style={{ alignItems: "center", justifyContent: "center" }}>
      <div style={{ fontSize: 120, fontWeight: 800, opacity: p, transform: `translateY(${(1 - p) * 40}px)` }}>
        {props.title}
      </div>
    </Fill>
  );
}
```

The item field is `codeComposition`:

- `{ source, width?, height?, assets?, props? }`: your own TSX source, or
- `{ template, props, assets? }`: a built-in template (`lower-third`, `title-card`, `counter`, `progress-bar`,
  `quote-card`, `logo-sting`, `kinetic-text`, `end-card`). Missing props take the template's defaults.

`width`/`height` default to the project's reference frame (1920×1080 for 16:9, 1080×1920 for 9:16, …). The frame is
fitted into the canvas and can be moved, scaled and faded like any other layer.

## Input

| name | meaning |
|---|---|
| `time` | seconds since the item started |
| `duration` | item length in seconds |
| `progress` | `time / duration`, 0…1 |
| `frame`, `fps` | frame number and project frame rate |
| `width`, `height` | composition size in CSS pixels |
| `props` | the composition's `props` (templates: merged with defaults) |
| `assets` | `assets.<name>` is an image source for `<img src={assets.<name>} />` |

## Language

TypeScript and JSX (types are stripped; nothing is type-checked). One file: **no `import`/`export … from`/dynamic
`import()`**. Everything else in modern JavaScript works (QuickJS-NG, ES2023).

Each frame runs in a **fresh sandbox** with:

- no file system, network, timers, `require`, `process`, DOM or `window`;
- `Date.now()` and `new Date()` pinned to 0, and `Math.random()` a fixed sequence restarting every frame, so a
  frame depends only on its inputs (use `random(key)` for stable per-element randomness);
- limits: 64 MB of memory, 250 ms of CPU, 1 MB of stack. Exceeding one is an error for that frame (`budget`,
  `memory`, or a stack-overflow `runtime` error); the layer is skipped and the rest of the frame renders.

## Helpers (globals)

| helper | meaning |
|---|---|
| `Fill` | full-frame flex column (`position: absolute; inset: 0`), the usual root |
| `clamp(v, lo = 0, hi = 1)`, `mix(a, b, t)` | clamping and linear interpolation |
| `interpolate(v, [in…], [out…], { easing, clamp })` | piecewise-linear mapping (clamped unless `clamp: false`) |
| `easing.linear/in/out/inOut/back/elastic/bounce`, `easing.cubicBezier(x1, y1, x2, y2)` | easing curves on 0…1 |
| `spring({ time, delay, from, to, stiffness, damping, mass })` | damped spring (closed form) |
| `stagger(time, index, step = 0.08, delay = 0)` | local time of the `index`-th staggered element |
| `random(key)` | stable pseudo-random number in [0, 1) for a key |
| `console.log/info/warn/error` | captured and shown by Check |
| `h`, `Fragment`, `React.createElement` | the JSX factory (components are plain functions) |

There are no hooks or state: compute everything from `time`.

## Elements

- Block: `div section article header footer main aside nav figure figcaption blockquote ul ol li p h1…h6`.
- Inline: `span strong b em i u s small code mark sub sup label a` (with the usual default styles).
- `img`, with `src={assets.name}` only (no URLs or file paths); `width`/`height` props or styles.
- `br`.
- Function components and fragments.

Not supported (reported by Check): `svg` and its shapes (use styled `div`s, `borderRadius`, gradients, or an image
asset), `video`, `audio`, `canvas`, `iframe`, `script`, `style`, `link`; other unknown tags render as `div`.
Event handlers and refs are ignored.

Props kept: `style` (object of camelCase CSS), `tw` (Tailwind classes), `id`, `className`, `src`, `width`,
`height`, `alt`.

## CSS

Layout and paint are by Takumi (CSS block, inline and flex layout, absolute positioning). Commonly used and tested:
`display` (block/inline/flex/none), flexbox (`flexDirection`, `flexWrap`, `justifyContent`, `alignItems`, `gap`),
`position`/`left`/`top`/`right`/`bottom`, sizes and `margin`/`padding`, `border`, `borderRadius`, `backgroundColor`,
`backgroundImage` (linear/radial gradients), `boxShadow`, `opacity`, `transform` (translate/scale/rotate),
`overflow`, `filter` (e.g. `blur()`), `color`, `fontFamily`, `fontSize`, `fontWeight`, `fontStyle`, `lineHeight`,
`letterSpacing`, `textAlign`, `textShadow`, `textTransform`, `textDecoration`, `whiteSpace`, `objectFit`,
`WebkitTextStroke`. Numbers are pixels.

The root defaults to Inter 48 px white text. Note that, unlike some web-to-image renderers, a plain `div` is
`display: block`; use `Fill` or `display: "flex"` for flex layout.

Check reports style properties Takumi does not know (they are dropped) and values it cannot parse.

Fonts: any family from the font library (`assets/fonts/library`) or installed on the computer; unknown families
fall back to Inter.

## Not supported

Lottie, video assets inside compositions, audio, CSS keyframe animations driven by stylesheets (animate from
`time` instead), and per-frame state.

## Check

`check` (the Code tab and the assistant's `check_code` tool) transpiles the source, reports syntax errors, imports
and unavailable globals with line/column, then renders frames at the start, middle and end, reporting runtime
errors, unsupported elements/props/styles, console output and the slowest frame time.

## Third-party code

Layout and painting use Takumi (MIT OR Apache-2.0), the sandbox is QuickJS-NG through rquickjs (MIT), and TSX is
transpiled with oxc (MIT). Takumi's raster backend includes the `jpeg-encoder` crate, whose forward DCT is based on
the work of the Independent JPEG Group: **this software is based in part on the work of the Independent JPEG
Group.**
