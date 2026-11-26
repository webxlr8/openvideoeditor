# Tasks: add-motion-graphics

## 1. Implementation

- [x] 1.1 Sandbox, transpile, layout and paint pipeline with golden frames for every template.
- [x] 1.2 Escape attempts (fs, network, timers, imports, prototype tampering) refused in tests.
- [x] 1.3 Motion panel, Code tab, assistant `check_code` tool; verified in the running app.
- [x] 1.4 Templates laid out for 9:16 as well as 16:9. Templates read the frame size: titles and counters shrink to the width, the end card stacks its frames, the quote card and kinetic text fit the width, and the lower third stays above the bottom fifth on a tall frame. Verified: 9:16 golden for every template, a test that overlays do not run off the sides (it failed before the change), 16:9 goldens unchanged.
- [ ] 1.5 Lottie import (not supported).
