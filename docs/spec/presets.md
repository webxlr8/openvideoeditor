# Preset catalogs (reference behaviour)

Identifiers and parameter contracts only. Shader source, artwork and copy are not included; reimplement looks from
the parameter contracts.

## Shader effects (41)

All run as a full-frame fragment pass over the layers beneath, with `amount`, `fadeIn`/`fadeOut` and
`region` (all | subject | background, using the person matte). Params with ranges:

### stylize
- `duotone` — shadows:color=[0.09, 0.11, 0.36]; highlights:color=[1, 0.55, 0.42]; balance:percent[0..1]=0.5
- `tritone` — shadows:color=[0.11, 0.122, 0.302]; midtones:color=[0.831, 0.337, 0.416]; highlights:color=[1, 0.914, 0.722]; midpoint:percent[0..1]=0.5
- `gradient-map` — shadows:color=[0.102, 0.043, 0.18]; midtones:color=[0.914, 0.271, 0.376]; highlights:color=[0.976, 0.929, 0.412]; speed:number[-2..2]=0.15; contrast:number[0..3]=1; blackPoint:percent[0..1]=0; whitePoint:percent[0..1]=1; strength:percent[0..1]=1
- `posterize` — levels:number[2..20]=5
- `solarize` — threshold:percent[0..1]=0.5; strength:percent[0..1]=1
- `halftone` — frequency:number[10..300]=60; angle:number[0..360]=45; cmyk:number[0..1]=0; misprint:number[0..0.01]=0
- `dither` — pixelSize:number[1..20]=4; threshold:percent[0..1]=0.5; spread:percent[0..1]=1; dark:color=[0, 0, 0]; light:color=[1, 1, 1]; sourceColors:number=0
- `engraving` — frequency:number[20..300]=90; angle:number[0..360]=8; crossHatch:number=1; relief:number[0..2]=0.6; waviness:percent[0..1]=0.35; contrast:number[0.25..3]=1.15; inkColor:color=[0.102, 0.078, 0.063]; paperColor:color=[0.957, 0.933, 0.886]
- `watercolor` — brushSize:number[1..12]=4; bleed:number[0..3]=1; paper:percent[0..1]=0.35; paperColor:color=[0.984, 0.969, 0.925]; strength:percent[0..1]=1
- `pixelate` — cells:number[1..200]=50; gap:percent[0..0.95]=0; roundness:percent[0..1]=0
- `chromatic-aberration` — strength:percent[0..1]=0.2; angle:number[0..360]=0; redOffset:number[-2..2]=-1; greenOffset:number[-2..2]=0; blueOffset:number[-2..2]=1
- `glitch` — intensity:percent[0..1]=0.5; speed:number[0..5]=1; rgbShift:number[0..20]=5; blockDensity:number[2..50]=10; colorBarIntensity:percent[0..1]=0.2; mirrorAmount:percent[0..1]=0.3; scanlineIntensity:percent[0..1]=0.2; seed:number[0..100]=3.8
- `vhs` — wobble:number[0..5]=1; scanlineNoise:percent[0..1]=0.6; smear:number[-2..2]=0.2; speed:number[0..3]=1
- `crt-screen` — pixelSize:number[8..128]=128; colorShift:number[0..10]=1; scanlineIntensity:percent[0..1]=0.3; scanlineFrequency:number[100..800]=200; brightness:number[0.5..2]=1; vignetteIntensity:percent[0..1]=1; vignetteRadius:percent[0..1]=0.5
- `light-leak` — position:point=[0.95, 0.6]; spread:number[0.1..1.5]=0.55; intensity:percent[0..1]=0.15; streaks:percent[0..1]=0.6; hotColor:color=[1, 0.953, 0.769]; midColor:color=[1, 0.478, 0.184]; fringeColor:color=[0.651, 0.239, 0.561]; speed:number[0..4]=1
- `lens-flare` — position:point=[0.3, 0.7]; intensity:percent[0..2]=0.5; ghosts:percent[0..1]=0.4; halo:percent[0..1]=0.4; starburst:percent[0..1]=0.3; streak:percent[0..1]=0.15; speed:number[0..3]=0.5
- `zoom-blur` — intensity:number[0..100]=30; center:point=[0.5, 0.5]
- `tilt-shift` — intensity:number[0..100]=50; width:percent[0..1]=0.3; falloff:percent[0..1]=0.3; angle:number[0..360]=0; center:point=[0.5, 0.5]

### distort
- `bulge` — center:point=[0.5, 0.5]; strength:number[-1..1]=0.6; radius:number[0.1..3]=1; falloff:percent[0..1]=0.5
- `spherize` — radius:number[0.1..3]=1; depth:number[0..3]=1; center:point=[0.5, 0.5]; lightPosition:point=[0.3, 0.7]; lightIntensity:percent[0..1]=0.5; lightSoftness:percent[0..1]=0.5; lightColor:color=[1, 1, 1]
- `lens-distortion` — bulge:number[-1..1]=0; roundness:percent[0..1]=0; spread:percent[0..1]=0.6; angle:number[0..360]=0; swirl:number[-1..1]=0.35; clearCenter:percent[0..1]=0.8; center:point=[0.5, 0.5]
- `twirl` — center:point=[0.5, 0.5]; intensity:number[-5..5]=0.5
- `wave-distortion` — strength:percent[0..1]=0.3; frequency:number[0.1..10]=1; speed:number[0..5]=1; angle:number[0..360]=0
- `kaleidoscope` — center:point=[0.5, 0.5]; segments:number[2..24]=6; angle:number[0..360]=0
- `mirror` — center:point=[0.5, 0.5]; angle:number[0..360]=0
- `polar-coordinates` — center:point=[0.5, 0.5]; wrap:number[0..2]=1; radius:number[0..2]=1; intensity:percent[0..1]=1
- `fluted-glass` — frequency:number[1..20]=10; angle:number[0..360]=0; refraction:number[0..4]=1.5; softness:percent[0..1]=0.5; aberration:percent[0..1]=0.2; highlight:number[0..2]=0.2; speed:number[-1..1]=0
- `glass-tiles` — intensity:number[0..10]=2; tileCount:number[5..50]=20; rotation:number[0..360]=0; roundness:percent[0..1]=0

### background
- `aurora` — sky:color=[0.02, 0.039, 0.102]; colorA:color=[0.647, 0.2, 0.973]; colorB:color=[0.133, 0.933, 0.533]; colorC:color=[0.086, 0.58, 0.91]; intensity:percent[0..1]=0.8; height:percent[0.1..2]=1.2; waviness:percent[0..2]=0.5; speed:number[-10..10]=5
- `mesh-gradient` — color1:color=[0.102, 0.02, 0.2]; color2:color=[0.427, 0.184, 0.82]; color3:color=[0.878, 0.294, 0.62]; color4:color=[1, 0.549, 0.259]; color5:color=[1, 0.875, 0.557]; smoothness:number[0..5]=2; swirl:number[-1..1]=0.3; speed:number[0..10]=1
- `flowing-gradient` — colorA:color=[0.039, 0, 0.082]; colorB:color=[0.42, 0.09, 0.902]; colorC:color=[1, 0.302, 0.416]; colorD:color=[1, 0.42, 0.208]; distortion:number[0..2]=0.5; seed:number[0..100]=0; speed:number[0..10]=1
- `plasma` — colorA:color=[0.439, 0.094, 0.745]; colorB:color=[0, 0, 0]; density:number[0..4]=2; warp:percent[0..1]=0.4; intensity:number[0.1..3]=1.5; balance:percent[0..1]=0.5; speed:number[0..5]=2
- `swirl` — colorA:color=[0.071, 0.459, 0.847]; colorB:color=[0.882, 0.569, 0.212]; detail:number[0..5]=4; blend:percent[0..1]=0.45; speed:number[0..5]=1
- `marble` — colorA:color=[0.95, 0.93, 0.89]; colorB:color=[0.7, 0.62, 0.5]; colorC:color=[0.25, 0.22, 0.2]; scale:number[0.1..10]=0.8; turbulence:number[0..50]=20; seed:number[0..100]=0; speed:number[0..0.25]=0.05
- `godrays` — center:point=[0, 1]; rayColor:color=[0.259, 0.514, 0.984]; background:color=[0, 0, 0]; density:percent[0..1]=0.3; intensity:percent[0..1]=0.8; spotty:percent[0..1]=1; speed:number[0..2]=0.5
- `sunburst` — color:color=[1, 0.867, 0.533]; background:color=[0, 0, 0]; center:point=[0.5, 0.5]; rayCount:number[3..64]=12; softness:percent[0..1]=0.3; radius:number[0..1.2]=0.8; feather:percent[0..1]=0.5; speed:number[-2..2]=0.2
- `spiral` — colorA:color=[0.06, 0.07, 0.16]; colorB:color=[0.98, 0.84, 0.58]; strokeWidth:percent[0..1]=0.5; strokeFalloff:percent[0..1]=0.4; softness:percent[0..1]=0.15; scale:number[0.1..5]=3; center:point=[0.5, 0.5]; speed:number[-3..3]=1
- `conic-gradient` — colorA:color=[1, 0, 0.502]; colorB:color=[0, 0.749, 1]; center:point=[0.5, 0.5]; rotation:number[0..360]=0; repeat:number[1..24]=1
- `voronoi` — colorA:color=[0.192, 0.525, 0.812]; colorB:color=[0.988, 0.008, 0.867]; borderColor:color=[0, 0, 0]; scale:number[1..20]=6; edgeIntensity:percent[0..1]=0.5; edgeSoftness:number[0..0.4]=0.05; speed:number[0..5]=0.5
- `stripes` — colorA:color=[0, 0, 0]; colorB:color=[1, 1, 1]; angle:number[0..360]=45; density:number[1..30]=5; balance:percent[0..1]=0.5; softness:number[0..0.5]=0; speed:number[-2..2]=0.2
- `studio-background` — color:color=[0.847, 0.859, 0.925]; lightColor:color=[0.835, 0.894, 0.918]; intensity:percent[0..1]=0.4; softness:percent[0..1]=0.5; center:point=[0.5, 0.2]; vignette:percent[0..1]=0; speed:number[-5..5]=2

## Caption styles (21; default `butter-serif`)

Each preset defines font family/size/weight, text and highlight colour, background + opacity, position (top/center/
bottom) with offsets, optional responsive placement and font scale per orientation, line height, letter spacing,
text transform, animation mode (word-by-word, stacked groups, pop, pulse…), reveal duration, shadow, optional frame
outline, and composition (front / behind-subject).

- `landmark` — center, 118px, weight 400, anim stacked, behind-subject
- `margin-type` — center, 108px, weight 700, anim stacked, behind-subject
- `poster-stack` — center, 104px, weight 800, anim stacked, behind-subject
- `soft-script` — center, 112px, weight 700, anim stacked, behind-subject
- `chrome-italic` — center, 116px, weight 900, anim stacked, behind-subject
- `scrapbook-cutout` — center, 106px, weight 900, anim stacked, behind-subject
- `marker-circle` — center, 118px, weight 700, anim stacked, behind-subject
- `neo-brutal` — center, 108px, weight 400, anim stacked, behind-subject
- `hero-world-wall` — center, 104px, weight 400, anim stacked, behind-subject
- `Cascade` — center, 50px, weight 500, anim reveal
- `rapid-reveal` — center, 96px, weight 800, anim reveal
- `stacked-emphasis` — center, 56px, weight 700, anim stacked
- `word-pop` — center, 88px, weight 600, anim word-pop
- `pulse-cut` — bottom, 62px, weight 850, anim word
- `beast-mode` — center, 72px, weight 400, anim word
- `butter-serif` — bottom, 52px, weight 400, anim word
- `minimal-modern` — bottom, 46px, weight 300, anim word
- `spotlight` — center, 60px, weight 700, anim spotlight
- `classic` — bottom, 40px, weight 500, anim word
- `subtitle` — bottom, 40px, weight 500, anim word
- `bold` — bottom, 60px, weight 800, anim word

## Colour

- LUT looks: `cinematic`, `black-and-white`, `vintage`, `cool`, `warm`, `high-contrast`, `faded-film`, `technicolor`, `newsreel`
- Adjust defaults: brightness=0, contrast=0, saturation=0, exposure=0, highlights=0, shadows=0, temperature=0, tint=0, vibrance=0, gamma=1
- Grade model: lift/gamma/gain/offset wheels (r,g,b,y), curves (master, RGB, hue-vs-sat/hue/lum, lum-vs-sat), saturation, LUT (preset or .cube, intensity, input|look stage), reference match.

## Layouts (15)

Normalised slot rectangles (x, y, w, h in 0–1) that clips are assigned into.

`split-horizontal` (split), `split-vertical` (split), `3up-horizontal` (split), `3up-vertical` (split), `4up-horizontal` (split), `4up-vertical` (split), `grid-2x2` (grid), `pip-bottom-right` (pip), `pip-bottom-left` (pip), `pip-top-right` (pip), `pip-top-left` (pip), `hero-top` (hero), `hero-bottom` (hero), `hero-left` (hero), `hero-right` (hero)

## Transitions

`audio-fade`, `chromatic-aberration`, `cross-zoom`, `dissolve`, `fade`, `film-burn`, `fly`, `glitch`, `ink-bleed`, `light-leak`, `mask-glitch`, `push`, `radial-wipe`, `wipe`, `zoom`, `push-left`, `push-right`, `push-up`, `push-down`, `wipe-left`, `wipe-right`, `wipe-up`, `wipe-down`, `scale`, `push-down`, `push-left`, `push-right`, `push-up`, `scale`, `wipe-down`, `wipe-left`, `wipe-right`, `wipe-up`

Each has duration and easing (linear, ease-in, ease-out, ease-in-out); directional ones take a direction.

## Text animations

Groups: basics (`none`, `fade`, `blur`), reveal (`typewriter`, `glitch`), move (`rise`, `drop`, `slide-left`, `slide-right`, `settle-up`, `settle-down`), scale (`zoom-in`, `zoom-out`, `pop`, `focus-pop`, `rotate-in`).

Each has duration and delay; swapping a style keeps text and placement.

## Keyboard shortcuts (27 actions)

| Action | Category | Default |
|---|---|---|
| Play / Pause | playback | `{ key: " " }` |
| Previous Frame | playback | `{ key: "ArrowLeft" }` |
| Next Frame | playback | `{ key: "ArrowRight" }` |
| Jump to Beginning | playback | `{ key: "ArrowLeft", meta: true }` |
| Jump to End | playback | `{ key: "ArrowRight", meta: true }` |
| Split at Playhead | timeline | `{ key: "b" }` |
| Delete Selected | timeline | `{ key: "Backspace" }` |
| Duplicate Selected | timeline | `{ key: "d", meta: true }` |
| Copy | timeline | `{ key: "c", meta: true }` |
| Cut | timeline | `{ key: "x", meta: true }` |
| Paste | timeline | `{ key: "v", meta: true }` |
| Mute Selected | timeline | `{ key: "m" }` |
| Remove Silences | timeline | `{ key: "s", shift: true }` |
| Undo | timeline | `{ key: "z", meta: true }` |
| Redo | timeline | `{ key: "z", meta: true, shift: true }` |
| Select All | timeline | `{ key: "a", meta: true }` |
| Toggle Snapping | timeline | `{ key: "s" }` |
| Toggle Keyframe Lanes | timeline | `{ key: "u" }` |
| Export | general | `{ key: "e", meta: true }` |
| Export Frame | general | `{ key: "e", meta: true, shift: true }` |
| Import Media | general | `{ key: "i", meta: true }` |
| Toggle Fullscreen | playback | `{ key: "f" }` |
| Toggle Chat Panel | general | `{ key: "]" }` |
| Toggle Left Panel | general | `{ key: "[" }` |
| Toggle Timeline | general | `{ key: "\\" }` |
| Focus Chat | general | `{ key: "/" }` |
| Search Projects | general | `{ key: "f", meta: true }` |

Presets override defaults: Default (none), Premiere-style (Home/End jumps, ⌘K split, ⌘M export, ` fullscreen, ⇧+ keyframe lanes), Resolve-style (⌘\ split, N snapping, P fullscreen), Final Cut-style (⌘B split, N snapping, ⌘⇧F fullscreen), CapCut-style (⌘B split, ⇧M mute, N snapping). Users can rebind any action.

## Fonts

Top-100 Google font families with full weight/italic files; featured list mixes Google families with display fonts.
Families: Roboto, Open Sans, Google Sans, Inter, Montserrat, Poppins, Lato, Noto Sans JP, Arimo, Roboto Mono, Roboto Condensed, Noto Sans, Oswald, Raleway, Nunito, Nunito Sans, DM Sans, Playfair Display, Rubik, Roboto Slab, Noto Sans KR, Ubuntu, Archivo Black, Kanit, Merriweather, Manrope, Outfit, Work Sans, Noto Sans TC, PT Sans, Lora, Bebas Neue, Prompt, Mulish, Black Ops One, Saira, Quicksand, Inconsolata, IBM Plex Sans, Source Sans 3, Figtree, Barlow, Bricolage Grotesque, Plus Jakarta Sans, Share Tech, Fira Sans, Smooch Sans, Karla, Titillium Web, Jost, Heebo, Archivo, Noto Serif, PT Serif, Source Code Pro, Dancing Script, Noto Sans SC, Fjalla One, Libre Baskerville, JetBrains Mono, Lobster Two, Changa One, Libre Franklin, Noto Serif JP, Barlow Condensed, Cormorant Garamond, Josefin Sans, Space Grotesk, Anton, Cairo, Nanum Gothic, Public Sans, Instrument Serif, EB Garamond, Alfa Slab One, Assistant, Sora, Schibsted Grotesk, Roboto Flex, Ramabhadra, Noto Sans Telugu, Cabin, Overpass, Inter Tight, Bitter, Anek Telugu, M PLUS Rounded 1c, Dosis, Noto Sans Khmer, Red Hat Display, Lexend, Exo 2, Oxygen, Fredoka, Hind Siliguri, Rajdhani, Caveat, Google Sans Flex, Urbanist, Slabo 27px

## Generation models (at capture time)

- Video: `kling-2.5-turbo-pro` (standard, [5, 10]s), `veo-3.1` (best, [4, 6, 8]s), `wan-3.0` (best, [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30]s), `gemini-omni-flash` (best, [3, 4, 5, 6, 7, 8]s), `gemini-omni-flash-1.2` (best, [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]s), `minimax-h3` (standard, [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]s), `hailuo-02-standard` (standard, [6, 10]s), `kling-3-standard` (standard, [3, 4, 5, 6, 7, 8, 9, 10]s), `kling-3-pro` (best, [3, 4, 5, 6, 7, 8, 9, 10]s), `seedance-2.5` (best, [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30]s), `seedance-2.0` (best, [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]s), `grok-imagine-1.5` (best, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]s)
- Image (via fal endpoints): `nano-banana` (standard), `gpt-image-2.5-sunburst` (best), `seedream-5-lite` (standard), `nano-banana-2.1` (standard)
- Voice: ElevenLabs (`eleven_v3`), library + clones. Audio processing: voice-cleanup, remove-music, split-vocals.
