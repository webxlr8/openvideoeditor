# Look LUTs

Nine looks as 17³ `.cube` files: `cinematic`, `black-and-white`, `vintage`, `cool`, `warm`, `high-contrast`,
`faded-film`, `technicolor`, `newsreel`.

They are written by `cargo run -p ove-render --example gen_luts` from the procedural looks in
`crates/ove-render/src/color.rs`, which the renderer evaluates directly at 33³. The looks are our own; the files are
dedicated to the public domain under CC0-1.0 (see `LICENSE`), so they may be used in any tool.

A grade picks one with `colorGrade.lut.preset` (or an adjustment item's `lutPresetId`); imported `.cube` files are
copied into the project bundle and referenced by `colorGrade.lut.file`.
