# Design: fix-render-and-edit-correctness

## Context

See proposal.md (Why). Each fix is small and local; the design records the choices that later changes depend on.

## Goals / Non-Goals

**Goals:**
- Output that matches the schema for every value it accepts.
- Split and trim that leave every frame, animated value and caption word at the same timeline time.
- No new behaviour beyond what the schema, specs and labels already promise.

**Non-Goals:**
- New editing operations (ripple, slip, roll: add-edit-planners-and-links).
- GPU parity (add-gpu-compositor-and-scopes keeps parity with the CPU formulas chosen here).

## Decisions

### D1. Blend formulas: W3C Compositing and Blending Level 1
Separable modes use the published per-channel formulas; hue, saturation, color and luminosity use the non-separable
definitions (luminance weights 0.3/0.59/0.11, clip-to-gamut). Blending works on straight-alpha 8-bit values as the
rest of the CPU compositor does. `add` stays as a legacy alias. Alternatives: the formulas of a specific editor
(undocumented, would need reverse engineering); linear-light blending (correct for HDR work but changes Normal output;
that belongs to add-color-management-and-subject-mattes).

### D2. One retiming module in ove-core
`ove-core::retime` holds split and start-edge rebase as pure functions on an item that return fields; callers turn
them into ops so each edit stays one revision. The renderer's keyframe easing moves into the same module so cut
keyframes and rendering cannot drift apart. Alternatives: fix the two desktop commands in place (the CLI, agents and
the coming planners would each re-implement it); create the `ove-edit` crate now (that is the next change's scope and
needs its design).

### D3. Keyframes at the cut
Each part keeps the keyframes inside its range, shifted to start at 0, plus a keyframe at the cut holding the value
there. For linear tracks the curve is unchanged; for eased tracks the end values are exact and the shape between the
cut and the next keyframe is re-eased. Alternatives: per-keyframe easing (a schema change owned by
add-markers-keyframe-lanes-and-versions); keeping the full track on both parts with an offset field (new schema field
for one fix).

### D4. Reverse playback
With reverse playback the start edge shows the latest source frame. Split gives the right part the earlier source and
the left part a later `trimStart`; trimming the end edge moves `trimStart`; extending the start edge is limited by the
source length. This matches the renderer's source mapping.

### D5. Encoders in the LGPL build
Enable `h264_nvenc`/`hevc_nvenc` (NVIDIA headers, MIT), `h264_amf`/`hevc_amf` (AMF headers, MIT), `h264_qsv`/
`hevc_qsv` (oneVPL dispatcher, MIT) on Windows and `libopenh264` (BSD-2) on both systems. None needs
`--enable-nonfree` or `--enable-gpl`; `check-license.sh` stays the gate. Alternatives: remove the names from the
candidate list (export would still fall back to MPEG-4 Part 2 on machines whose Media Foundation encoder fails);
x264 (GPL, not allowed).

### D6. Resume by the provider's remote handle
At submit time the job stores the provider's status, result and cancel addresses. When the worker that ran a job is
gone but the handle exists, the job becomes queued again and a worker resumes polling without submitting a second
time. The overall timeout counts from the first submission, so a resume cannot extend it. The same domain checks apply
to stored addresses as to fresh ones. Alternatives: fail as today (loses paid results); re-submit (pays twice).

## Risks / Trade-offs

- Existing projects that used `color-dodge` render differently (they rendered as Add) → the change is a fix to the
  schema's meaning; release notes mention it. Detection: golden render of a fixture with each blend mode.
- Effect and transition golden hashes may change where blend modes are used → re-bless only after checking the new
  frames by eye; detection: the golden tests fail with the mode named.
- The new encoders add DLLs and size → measure in `sizes.txt`; detection: the installer size check in packaging.
- Vendor encoders fail on machines without the GPU → they are tried in order and skipped on open failure, as today;
  detection: export tests with each encoder forced on the Windows test machine.
- A resumed job polls a request the provider already expired → the provider's error fails the job with its message;
  detection: stand-in server test for an expired request.

## Open Questions

- FFmpeg rebuilds with the new encoders must run on the owner's macOS and Windows build machines; this machine has no
  MSYS2 toolchain. Default: the build.sh change lands with the code; the binaries are rebuilt before the next release.
- OpenH264 is built from source (BSD-2-Clause). Cisco's patent licence for H.264 covers only the binaries Cisco itself
  distributes. Should the installer ship our build, or download Cisco's binary on first use with the person's consent?
  Default: ship our build in development builds; the owner decides before the first public release.
- Should the effect golden hashes be per-OS tables or a tolerance comparison? Default: tolerance comparison (max
  channel difference ≤ 1), so the same table serves all three systems.

## Results

- Blend modes: `every_schema_blend_mode_is_rendered` and `blend_formulas` pass (`cargo test -p ove-render --lib
  canvas`, Windows 11, 2026-11-26).
- Retiming: 7 `ove-core::retime` tests pass (Windows 11, 2026-11-26).
- Desktop: `edits` tests incl. `reverse_trim_moves_the_other_source_edge` pass (`cargo test -p ove-desktop --lib`).
- Generation resume: `cargo test -p ove-generate` passes, incl. `tests/fal_resume.rs` (resume without a second
  submission, timeout cancels at the provider, foreign status address refused); clippy clean.
- Not run here: the 3 `ove-render` mixdown tests need a full FFmpeg CLI for test tones (`OVE_FIXTURE_FFMPEG`).
- Blend-mode golden (task 1.2, `crates/ove-render/tests/blend_golden.rs`): a 4×4 grid on a 16:9 canvas with one
  code-item layer per schema blend mode over a shared backdrop ramp. It renders at 320×180 and is stored as
  `tests/golden/blend-modes-320x180.pam`. The frame was checked by eye before blessing. Each cell shows its mode's
  expected result: darker for multiply and burn, lighter for screen, lighten and dodge, inversion for difference, and
  backdrop luminance with the source hue for hue and color. A second test renders every cell with `normal` and
  requires a difference above 8 steps. Mutation check: mapping `luminosity` to Normal fails both tests ("blend output
  changed: luminosity (max channel difference 219)", "these modes render as normal: luminosity"). Windows 11,
  2026-11-27.
- Effect goldens (task 1.3, `crates/ove-render/tests/effects_golden.rs`): stored frames replace the Linux-only FNV
  hash table. The file `tests/golden/effects-48x27.pam` holds the 41 effects stacked. The test compares with a
  tolerance of one 8-bit step per channel and has no OS condition. Before the change, the old hash test ran once on
  Windows with its condition removed: all 41 hashes equalled the table blessed on Linux x86-64, so the two systems
  give identical frames today. Measured on Windows: max channel difference 0 for all 41 effects and all 16 blend
  cells. macOS and Linux were not run on this machine (no macOS host, no Rust in the WSL distro); the CI matrix runs
  them.
- `cargo test -p ove-render` on Windows 11 (2026-11-27): lib 47, blend_golden 2, captions_golden 3 (1 ignored),
  effects_golden 2, export_safety 6, mixdown 3, motion 5 and render 6 passed. The HDR fixture in `render.rs` needs
  python3 and ran with `OVE_ALLOW_MISSING_MEDIA_TOOLS=1` (reported as skipped).
- Split property test (task 2.3, `crates/ove-edit/tests/split_property.rs`): 10,000 random video and audio items
  (speed 0.25–4, forward and reverse, trimStart 0–30 s, optional trimEnd, 0–3 keyframe tracks with 1–6 keyframes,
  some before the start or after the end, all four easings), each split at a random time inside it. At 200 evenly
  spaced times plus the cut and every keyframe, the part that covers the time shows the same source time as the
  original, within 3 µs (the rounding of the stored fields). Linear tracks give the same value at every sampled time;
  eased tracks give the same value at the cut and at their own keyframes (D3). 10,000 cases run in 3.2 s. Mutation
  check: treating reversed items as forward fails at once ("source at 0: 25000 vs 0.25").
- Split e2e (task 2.4, `apps/desktop/e2e-app/flows.mjs`, "split export"), built debug app on Windows 11: a 4 s clip
  is set to 2× with the Inspector's speed chips and gets a 0.5 s fade in and fade out from the Transitions panel. It
  is exported at 720p, split a quarter of the way in (1.000 s), and exported again. The fades stay on their sides and
  the right part keeps the speed. The two exports compare with SSIM 1.00000 within 0.4 s of the cut and 1.00000 over
  the whole export (full FFmpeg CLI, `ssim` filter). Flow time 67.8 s. Note: a speed change keeps the clip's length,
  so at 2× a 4 s source runs out after 2 s of timeline; the cut is where both sides still show moving source.
- Encoders (tasks 3.1–3.3): `tools/ffmpeg/build.sh` builds OpenH264 2.7.0 (shared, all targets). On Windows it adds
  NVENC (nv-codec-headers n12.1.14.0), AMF (headers 1.5.3) and QSV (libvpl 2.17.0, static). All sources are pinned
  by SHA-256. It enables `libopenh264`, `h264/hevc_nvenc`, `h264/hevc_amf` and `h264/hevc_qsv`, copies the licence
  texts into `out/<target>/licenses` (shipped by `tools/notices.sh`), and fails when `ffmpeg -encoders` lacks one of
  the encoders. `tools/package.sh` ships the OpenH264 library. `check-license.sh` also refuses the CUDA compiler,
  CUDA SDK, NPP, DeckLink and more GPL-only libraries. It passes on the current Windows build configuration and fails
  on a sample with `--enable-cuda-nvcc --enable-libnpp`. Not built here: this machine has no MSYS2, so the sizes,
  the `ffmpeg -encoders` check and the forced-encoder exports are open.
- Still images: the e2e run showed that the LGPL build cannot open PNG files (`ffmpeg -i image.png` gives "Invalid
  data found when processing input"). The demuxer list had no image demuxer, so generated images and imported
  pictures do not decode in preview or export. `build.sh` now enables the `image2`, `png_pipe` and `jpeg_pipe`
  demuxers. This also needs the rebuild.
- Generation resume (task 4.3), command line (`crates/ove-cli/tests/generate.rs`,
  `a_killed_worker_is_resumed_without_a_second_submission`): `ove generate image --no-wait` runs against a local
  stand-in provider that stays IN_PROGRESS. The background worker is killed (`taskkill /F`) while it polls. The job's
  last update is then moved back past the one-minute grace instead of waiting. The stand-in completes, and `ove
  generate status --wait` resumes the job with a new worker. Result: succeeded, one submission, output in the bundle,
  placeholder filled; 2.5 s.
- Generation resume, app (`flows.mjs`, "generate resume"), Windows 11: Generate panel, confirm, the job runs at a
  local stand-in (`OVE_FAL_QUEUE_URL`), the app process is killed, the stand-in completes, the app is relaunched and
  the project opened. The job resumed after the one-minute grace (55 s after reopening) and succeeded. The stand-in
  received one submission, and the placeholder was filled with the generated media. Flow time 68.7 s.
- App e2e whole-run log check: it fails on this machine because the shared Windows profile holds log records and
  crash reports from 2026-11-26 (forced panics from the diagnostics page, written before these runs). The log records
  of the runs on 2026-11-27 have no ERROR or panic lines.
