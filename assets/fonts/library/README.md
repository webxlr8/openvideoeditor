# Font library

Open-licence families bundled with the app for timeline text, captions and motion-graphics code items. Every family
here is licensed under the **SIL Open Font License 1.1**; each directory holds the font files and that family's
`OFL.txt` (copyright notice and licence), which must ship alongside the fonts. The files are unmodified copies from
the families' upstream releases as published in the public font collection at github.com/google/fonts (`ofl/`).

| Family | Directory | Files | Weights | Italic | Category |
|---|---|---|---|---|---|
| Inter | `inter/` | variable (`opsz`, `wght`) | 100–900 | yes | sans-serif |
| Roboto | `roboto/` | variable (`wdth`, `wght`) | 100–900 | yes | sans-serif |
| Montserrat | `montserrat/` | variable (`wght`) | 100–900 | yes | sans-serif |
| Poppins | `poppins/` | static | 400, 600, 700, 900 | 400 | sans-serif |
| DM Sans | `dmsans/` | variable (`opsz`, `wght`) | 100–1000 | no | sans-serif |
| Space Grotesk | `spacegrotesk/` | variable (`wght`) | 300–700 | no | sans-serif |
| Playfair Display | `playfairdisplay/` | variable (`wght`) | 400–900 | yes | serif |
| Lora | `lora/` | variable (`wght`) | 400–700 | yes | serif |
| Instrument Serif | `instrumentserif/` | static | 400 | yes | serif |
| Bebas Neue | `bebasneue/` | static | 400 | no | display |
| Oswald | `oswald/` | variable (`wght`) | 200–700 | no | display |
| Anton | `anton/` | static | 400 | no | display |
| Archivo Black | `archivoblack/` | static | 400 | no | display |
| JetBrains Mono | `jetbrainsmono/` | variable (`wght`) | 100–800 | no | mono |
| Caveat | `caveat/` | variable (`wght`) | 400–700 | no | handwriting |
| Dancing Script | `dancingscript/` | variable (`wght`) | 400–700 | no | handwriting |

The catalogue in code is `crates/ove-graphics/src/fonts.rs` (`FACES`). Fonts are resolved by family, weight and
style: bundled families first (closest weight; the `wght` axis is set on variable fonts), then fonts installed on
the computer (via `fontdb`), then Inter. Set `OVE_NO_SYSTEM_FONTS=1` to ignore installed fonts (reproducible
renders across machines).

The older `assets/fonts/Inter-Regular.otf`/`Inter-Bold.otf` remain the default face for text that names no
family, so existing projects render exactly as before.
