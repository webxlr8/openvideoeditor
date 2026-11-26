# Tasks: add-effects-and-color

## 1. Implementation

- [x] 1.1 Effects, transitions, layouts and grade in the renderer with tests.
- [x] 1.2 Panels and inspector editors; verified in the running app and an exported video.
- [x] 1.3 GPU path. Every effect has a WGSL port (`effects/wgsl/`) run as a compute shader on its own device for images of 640x360 and up; any failure falls back to the CPU, which gives the same picture. Verified: every port matches the CPU on the sample frame at default and moved parameters (mean difference at most 0.11 of 255, RTX 4070, DX12); at 1080p the slowest effect takes 13 ms instead of 56 ms.
- [ ] 1.4 Subject/background regions.
- [x] 1.5 Waveform and vectorscope scopes. Colour tab: Histogram, Waveform (luma by column, 0–100 % lines) and Vectorscope (hue and saturation, 75 % bar targets, skin tone line), drawn by the renderer from the frame at the playhead. Verified: renderer tests place levels and hues; UI test switches scopes; app e2e (effects flow) loads both pictures.
