# Tasks: fix-render-and-edit-correctness

## 1. Blend modes

- [x] 1.1 Map all 16 schema blend modes plus the legacy `add` in `ove-render` (separable and non-separable
      formulas); verify `every_schema_blend_mode_is_rendered` and `blend_formulas` pass in `cargo test -p ove-render`.
- [x] 1.2 Add a golden render fixture with one layer per blend mode; verify frames by eye, then bless; verify the test
      fails if any mode falls back to Normal.
- [ ] 1.3 Make the effect golden tests compare with a tolerance (max channel difference ≤ 1) and run them on macOS,
      Windows and Linux; verify `cargo test -p ove-render` passes on all three. (Implemented; passes on Windows.
      Open until the CI matrix shows it passing on macOS and Linux: neither can run on this machine.)

## 2. Split and trim

- [x] 2.1 Add `ove-core::retime` (split, start-edge rebase, shared easing) with unit tests for speed, reverse,
      keyframes, caption segments and edge fields; verify `cargo test -p ove-core retime` passes.
- [x] 2.2 Use it in the desktop `split_item` (collision-free right id, unset end-edge fields on the left) and
      `trim_item` (rebase keyframes and captions; reverse-aware `trimmed`); verify the `edits` unit tests, including
      `reverse_trim_moves_the_other_source_edge`, pass in `cargo test -p ove-desktop`.
- [x] 2.3 Property test: for random items (speed 0.25–4, both directions, random keyframes) the split parts give the
      same source time and keyframe value as the original at 200 sampled times; verify it passes 10,000 cases.
- [x] 2.4 UI e2e: split a sped-up clip with a fade in the built app; verify the exported frames either side of the cut
      match an export of the uncut clip (SSIM ≥ 0.99). (Windows built app; the app e2e job runs on Windows and
      Linux, and macOS is covered by 6.1.)

## 3. Export encoders

- [ ] 3.1 Enable NVENC, AMF and QSV encoders (with their MIT headers / dispatcher) on Windows and OpenH264 on both
      systems in `tools/ffmpeg/build.sh`; extend `check-license.sh`; verify the licence check passes and `ffmpeg
      -encoders` lists them on each build machine. (build.sh, check-license.sh and the README are written; build.sh
      now fails when an encoder is missing from `ffmpeg -encoders`. Open until the rebuild runs on the macOS and
      Windows build machines: this machine has no MSYS2.)
- [ ] 3.2 Record the size change in `sizes.txt` and the README; verify the installer stays within 60 MB. (Needs the
      rebuild from 3.1.)
- [ ] 3.3 Export with each encoder forced on the Windows test machine and with all hardware encoders disabled; verify
      H.264 output and no fallback warning while OpenH264 is present. (Needs the rebuild from 3.1.)

## 4. Generation resume

- [x] 4.1 Store the provider's remote handle on the job; resume polling when the handle exists; timeout from first submission;
      domain checks on stored addresses; verify stand-in server tests (one submission on resume, foreign domain refused,
      expired request fails).
- [x] 4.2 Reaping: dead-worker jobs with a handle become resumable (one claim per dead worker) and are restarted by
      the app and by `ove generate status|list`; others fail as before; verify unit tests in `job.rs` and
      `tests/fal_resume.rs`.
- [x] 4.3 Kill the worker mid-job in the built app and in the CLI against a stand-in server; verify the job resumes
      with one submission and fills its placeholder.

## 5. Labels and bookkeeping

- [x] 5.1 Audio panel labels say what the buttons do (already in commit 8b5eb09); verify `Editing.test.tsx` asserts
      them (vitest: 137 passed).
- [x] 5.2 Mark add-ai-generation task 1.2 open for its MCP part, pointing to unify-agent-tool-layer; verify by reading
      the task list.

## 6. Integration

- [ ] 6.1 In the built app on macOS and Windows: render the blend-mode fixture, split and trim a reversed, sped-up,
      keyframed clip, export with the hardware encoder disabled, close the app during a generation and reopen;
      verify each behaves as the spec states and record the results in design.md.
