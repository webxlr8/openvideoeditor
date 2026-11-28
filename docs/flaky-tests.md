# Timing-sensitive tests

These tests measure time on real threads or processes. On a loaded machine (a full parallel workspace run, or
other build agents on the same CPU) they failed although the code was correct. Gate 2 decided to fix each one, not
to loosen limits for all tests and not to quarantine them without notice.

CI runs these tests in a separate serial step (`test (timing-sensitive, serial)` in `.github/workflows/ci.yml`)
after the parallel run, with `--test-threads=1`. The parallel run skips them. The serial step fails when a test
fails or when a name no longer matches exactly one test. They still gate the build.

When you add or rename a test here, change the skips and the serial step in `ci.yml` too.

Review date: 2027-02-27. On that date, check each test below. If it has not failed for a load reason since the
fix, move it back into the parallel run and remove it from this note.

| Test | Why it is timing-sensitive | Fix |
| --- | --- | --- |
| `ove-cli` `tests/mcp_edits.rs` `two_sessions_are_told_apart_and_each_undoes_only_its_own_change` | Two `ove mcp` sessions edit one project. Each session keeps the project open for `HOLD` (2 s) after a call. The other session waited only `WAIT_FOR_SESSION` (3 s) for it. Under load the first session let go late, and the second got `project.locked`. This was a real bug, not only a test problem: two agents on a busy machine hit it too. | `WAIT_FOR_SESSION` is now 7 s (`HOLD` plus 5 s of margin). The unit test `the_wait_for_a_session_outlasts_its_hold` in `ove-cli/src/mcp.rs` checks `WAIT_FOR_SESSION >= HOLD + 5 s`. The test starts both sessions with `OVE_MCP_HOLD_MS=200` and checks that each rename is not an error. A second cause was found later and fixed: a lock race. A holder removes `.lock.holder` just before it unlocks, and writes it just after it locks. A waiter that polled between those steps found the lock busy and no holder file. It did not know the holder was `ove`, so it reported `project.locked` at once instead of waiting. Now `open_write_waiting` (`ove-tools/src/bundle.rs`) also waits while the holder file is missing or cannot be read. Only a holder that names another process (the app) is reported at once. The holder file is written with a temp file and a rename, so a waiter never reads half of it. The remove-then-unlock order stays: the other order could delete the next holder's file. `ove-tools/tests/lock_wait.rs` checks the wait, the error after `WAIT_FOR_SESSION` when the holder never writes its file, and that the app is reported at once. On a failure the test now prints the error code and how long the call took. |
| `ove-agent` `tests/agent.rs` `stop_cancels_mid_stream_and_keeps_history_valid` | The test stopped the turn from a thread after 300 ms and required the whole turn to end within 2 s of its start. Under load the thread, the mock server and the turn all ran late, so the bound measured the machine, not the stop. | The test now stops the turn at the first streamed text and measures from that moment. The bound is 1.5 s. The rest of the stream takes about 3 s (150 chunks, 20 ms apart, and longer under load), so a pass shows that the stop ended the stream. |
| `ove-tasks` `tests/scheduler.rs` `wait_times_out_with_the_remaining_count` | After the wait for the `p1` tasks, the test checked that no task at all was left to cancel. The `p2` task is not in that wait, so under load it was often still queued or running. | Fixed in `add-background-tasks-and-media-derivatives`: the test waits for all tasks before that check. The time-out checks are unchanged. |
| `ove-desktop` `src/scrub.rs` `scrub::tests::scrub_latency_and_edits` | It measures wall-clock time from a scrub event to a ready audio grain against the latency budget (cold p95 at most 120 ms, cached p95 at most 40 ms). A cold grain decodes and mixes audio on the CPU, so other tests on the same CPU make it slower. In a parallel run the cold p95 was 140 ms. Alone it passes. | Not changed. The limit is a product budget for latency that the person feels, so the test keeps wall-clock time and the same limits. CPU time would hide disk and scheduling delays. The serial step runs it with no other tests, which is the condition the budget is for. |
| Desktop UI test for the skim thumbnail (`SkimThumb`) | It depended on frame timing in the test web view. | Another worker moves it to the built-app check (app e2e). It is not in the serial step. |

## Activity flood frame time (ui/src/activity/Activity.test.tsx)

- **Why timing-sensitive:** it asserted a 16 ms median and 32 ms p95 draw time in jsdom with React's development
  build. Those wall-clock numbers depend on machine load, and it failed twice in a row while other builds ran
  (2026-11-28).
- **Fix:** same rule as SkimThumb. The unit test keeps the batching (fewer than 1,000/3 draws) and the 200-outline cap,
  and logs draw times without asserting them. The 16 ms frame budget moves to the built-app check
  (add-agent-chat-context-and-activity task 4.3).
- **Review date:** 2027-02-27.

