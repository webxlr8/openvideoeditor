# Spec Delta

## Purpose

Seeing and measuring colour: waveform, parade and vectorscope scopes for people, scope images for agents, and a
numeric colour readout that agents can use to match shots.

## ADDED Requirements

### Requirement: Visual scopes
The colour panel SHALL show a luma waveform, an RGB parade and a vectorscope with a skin-tone line and 75% colour
target boxes. Scopes SHALL be computed from the frame that playback shows and SHALL update at least 10 times per
second during playback.

#### Scenario: Colour bars
- **WHEN** the 75% colour bars test frame is shown
- **THEN** each bar's vector lands in its target box within 2 bins and the waveform shows each bar's level within
  1 bin

#### Scenario: Scopes while paused
- **WHEN** playback is paused and the person changes the grade
- **THEN** the scopes update to the new frame within 200 ms

### Requirement: Scope images for agents
`ove look` SHALL write waveform, parade or vectorscope images for a time when asked, using the same binning as the
panel.

#### Scenario: Vectorscope image
- **WHEN** `ove look <project> --at 12 --scope vectorscope --out v.png` is run
- **THEN** `v.png` contains the vectorscope of the composited frame at 12 seconds

#### Scenario: Unknown scope
- **WHEN** `--scope histogram2` is requested
- **THEN** the command exits with code 1 and lists the valid scope names

### Requirement: Numeric colour readout
`ove color inspect` and the MCP tool `inspect_color` SHALL report luma statistics, clipping, zone colours, colour
balance, saturation and hue distribution for a project time. The measurement SHALL use the exported pixels at that
exact frame. With an item named and isolation requested, only that item SHALL be measured.

#### Scenario: Readout fields
- **WHEN** inspect runs on any frame
- **THEN** the output has luma mean with 2nd and 98th percentiles, clipped-low and clipped-high percentages, a 16-bin
  luma histogram, mean colour of shadows, midtones and highlights, CIELAB a* and b* balance, mean saturation, a
  saturation-weighted 12-bin hue histogram, percent of colourful pixels and skin-line deviation

#### Scenario: Deterministic output
- **WHEN** the same inspect request runs twice on an unchanged project
- **THEN** both outputs are byte-identical

#### Scenario: Isolated item
- **WHEN** inspect runs with `--item <id> --isolate` on a frame where a title covers the clip
- **THEN** the readout describes the clip alone, without the title

#### Scenario: Time past the end
- **WHEN** inspect is asked for a time after the timeline ends
- **THEN** the command exits with code 1 and states the timeline duration

### Requirement: Reference matching
When a reference item or media frame is given, the readout SHALL add the per-zone colour difference in CIEDE2000 and a
`suggestedOps` patch that sets grade wheels and saturation to move the shot toward the reference. The patch SHALL be
directly applicable as one edit and SHALL be limited in strength.

#### Scenario: Agent loop converges
- **WHEN** a mock agent applies `suggestedOps` to a warm-shifted copy of a reference shot and inspects again, up to
  three times
- **THEN** the mean CIEDE2000 difference to the reference falls below 2

#### Scenario: Reference not found
- **WHEN** the reference names a media id that does not exist
- **THEN** the command exits with code 1 and names the missing reference
