# Spec Delta

## Purpose

Compositing on the GPU with the CPU path as the reference: same output for preview, agent frames and export, effects
that can write alpha and change over time, a chroma key, and backend selection with fallback.

## ADDED Requirements

### Requirement: CPU and GPU parity
The GPU and CPU backends SHALL produce matching frames for every blend mode, effect, transition, grade and LUT. For
each, on the test charts and reference frames at 1080p, the maximum channel difference SHALL be at most 2/255, the
mean difference at most 0.25/255, and the 99th percentile CIEDE2000 difference at most 1.0.

#### Scenario: Effect parity
- **WHEN** a frame with any one of the 41 effects is rendered with both backends
- **THEN** the two frames differ by no more than the stated tolerances

#### Scenario: Parity failure is reported
- **WHEN** a code change makes one transition exceed the tolerance
- **THEN** the parity test fails and names the transition and the measured difference

### Requirement: One compositor for every output
Preview, agent `look` frames and export SHALL use the same compositor and the same backend choice, so a frame shown to
a person, a frame shown to an agent, and the exported frame at the same time are the same within the parity
tolerance.

#### Scenario: Look matches export
- **WHEN** an agent requests a `look` frame at a time and the project is then exported
- **THEN** the exported frame at that time matches the `look` frame within the parity tolerance

### Requirement: Backend selection and fallback
The app SHALL use the GPU backend when a GPU is available and a startup self-test frame matches the CPU reference
within tolerance. Otherwise it SHALL use the CPU backend and show the reason in the diagnostics overlay. A setting and
a command-line option SHALL force either backend.

#### Scenario: Self-test fails
- **WHEN** the startup self-test frame differs from the CPU reference beyond tolerance
- **THEN** the app uses the CPU backend and the diagnostics overlay states that the GPU self-test failed

#### Scenario: Device lost during preview
- **WHEN** the GPU device is lost while the preview plays
- **THEN** preview continues on the CPU backend without closing the project and the event is logged

#### Scenario: Forced backend
- **WHEN** `ove render --backend cpu` is run on a machine with a working GPU
- **THEN** the render uses the CPU backend and the output reports it

### Requirement: Preview resolution
With the GPU backend active, timeline preview SHALL composite at up to 1080p and keep 30 fps for a project with four
video layers, a grade and two effects on the reference machines.

#### Scenario: Preview frame rate
- **WHEN** such a project plays for 60 seconds on a reference machine with the GPU backend
- **THEN** no frames are dropped

### Requirement: Effects can write alpha
An effect SHALL be able to change the alpha of its layer. Effects that do not use alpha SHALL leave it unchanged.

#### Scenario: Existing effect keeps alpha
- **WHEN** a semi-transparent image layer has a colour effect applied
- **THEN** its alpha is the same as without the effect

### Requirement: Keyframable effect parameters
Every numeric effect parameter SHALL accept keyframes, evaluated with the same easing rules as transform keyframes. A
parameter without keyframes SHALL behave as before.

#### Scenario: Animated blur
- **WHEN** a blur effect has its radius keyframed from 0 to 20 over one second
- **THEN** the frame at the midpoint is blurred with radius 10 under linear easing

#### Scenario: Keyframe on unknown parameter
- **WHEN** a projection sets keyframes on a parameter the effect does not have
- **THEN** apply refuses the edit with a diagnostic naming the effect and the parameter

### Requirement: Chroma key
A `chroma-key` effect SHALL remove a key colour from its layer, with controls for tolerance, softness, spill
suppression, choke, matte black level, matte white level and edge blur, and a view that shows the matte instead of the
result. The key SHALL work on dark and desaturated screens.

#### Scenario: Green screen keyed
- **WHEN** `chroma-key` with a sampled green is applied to the green-screen test plate
- **THEN** the matte differs from the ground-truth matte by no more than the target sum of absolute differences, and
  edge spill (mean of max(0, G − max(R, B)) in the edge band) is below 0.02

#### Scenario: Matte view
- **WHEN** the matte view is on
- **THEN** the layer renders as a grey matte, and an agent `look` with the matte view returns the same matte

#### Scenario: Key colour not in frame
- **WHEN** the key colour does not appear in the layer
- **THEN** the layer renders unchanged and no error is raised

### Requirement: Key colour sampler
The viewer SHALL let the person click a point or drag a rectangle on the source to set the key colour. The sample SHALL
be the average source colour before grading. Setting the colour SHALL be one undoable revision.

#### Scenario: Area sample
- **WHEN** the person drags a rectangle over the green screen in the viewer
- **THEN** the key colour is set to the average of that area and one revision is created

#### Scenario: Sample outside the layer
- **WHEN** the person clicks outside the selected layer's bounds
- **THEN** the key colour does not change and the viewer shows that the point is outside the layer
