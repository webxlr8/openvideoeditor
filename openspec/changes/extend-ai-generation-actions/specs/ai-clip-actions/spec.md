# Spec Delta

## Purpose

Generation driven by a signed model catalog: several references per request, preprocessing of clip ranges, spending
kept under the person's control, AI actions on existing clips that land as takes or replace the source, and AI
transitions.

## ADDED Requirements

### Requirement: Model catalog with capabilities
The app SHALL describe each generation model with its modes, limits, prices, action tags and settings, and SHALL offer
only the modes and settings a model supports. The app SHALL work with its embedded catalog when offline.

#### Scenario: Offline start
- **WHEN** the app starts with no network
- **THEN** the Generate panel lists the models of the embedded or cached catalog

#### Scenario: Unsupported mode hidden
- **WHEN** a person selects a model without first + last frame mode
- **THEN** the panel does not offer a last-frame input

### Requirement: Signed remote catalog
A remote catalog SHALL be used only when its signature verifies against a trusted key and its schema version is
supported; otherwise the previous catalog SHALL stay in use and a warning SHALL be logged. Fetching it SHALL NOT send
any key, user identity or project data, and SHALL happen at most once a day unless the person asks.

#### Scenario: Tampered catalog ignored
- **WHEN** the remote catalog's content does not match its signature
- **THEN** the app keeps the previous catalog and logs a warning, and no model changes

#### Scenario: Endpoint outside the allowed domains
- **WHEN** a signed catalog entry names an endpoint on a domain that is not allowed for that provider's key
- **THEN** that entry is rejected and the other entries load

### Requirement: References in requests
A request SHALL accept several image, video and audio references within the model's limits. The prompt SHALL address
references as `@Image1`, `@Video1`, `@Audio1` in the order they were added. A tag without a matching reference, or a
reference count above the model's limit, SHALL be refused before any spending.

#### Scenario: Two references
- **WHEN** a person generates a video with prompt "@Image1 walks into @Image2" and two image references
- **THEN** both images are sent in order

#### Scenario: Missing reference refused
- **WHEN** a prompt contains `@Video1` and no video reference is attached
- **THEN** the request is refused with a message, and nothing is spent

### Requirement: Reference uploads and lineage
References SHALL be uploaded with the person's own provider key, and an upload SHALL be reused until it expires. The
generated media SHALL record its model, prompt, references, seed and settings.

#### Scenario: Lineage recorded
- **WHEN** a video generated from two image references finishes
- **THEN** its media record lists the model, the prompt and both reference media ids

#### Scenario: Upload reused
- **WHEN** the same reference file is used twice before its upload expires
- **THEN** it is uploaded once

### Requirement: Preprocessing of clip ranges
An action on a clip SHALL send only the clip's visible range by default. A request SHALL be able to use a composited
timeline span as a reference. Inputs SHALL be downscaled and transcoded to what the model accepts. Temporary files SHALL
be deleted when the job ends.

#### Scenario: Trimmed range only
- **WHEN** an upscale runs on a clip that shows 4 s of a 60 s source
- **THEN** the uploaded video is 4 s long

#### Scenario: Timeline span reference
- **WHEN** video-to-sound-effects runs on a timeline span with a title over footage
- **THEN** the uploaded reference shows the title over the footage

### Requirement: Spend control
Every paid call SHALL show an estimate before it runs. The app SHALL keep a local record of estimates and outcomes per
project. The person SHALL be able to set a budget per project and per agent session. A call that would exceed a budget
SHALL wait for the person's confirmation in the app, whoever requested it.

#### Scenario: Estimate shown
- **WHEN** a person starts an upscale of a 10 s clip
- **THEN** the estimate in USD and the price date are shown before confirming

#### Scenario: Agent over budget
- **WHEN** an agent requests a generation that would exceed its session budget
- **THEN** the call does not run until the person confirms in the app, and the agent receives a "needs confirmation"
  result

#### Scenario: Ledger records outcome
- **WHEN** a paid job fails at the provider
- **THEN** the project's spend record shows the job as failed with its estimate

### Requirement: AI actions on clips
For a selected clip, the app SHALL offer the actions its media type and the catalog support: upscale, reframe or
extend, lip sync, prompt edit, video to sound effects, video to music, dubbing, voice cleanup, voice changer, rerun and
draft to enhance, and SHALL show why an action is unavailable.

#### Scenario: Unavailable action explained
- **WHEN** a person opens the AI menu on an image clip
- **THEN** lip sync is disabled with the reason "needs a video clip"

### Requirement: Placement of action results
A visual result SHALL appear as a placeholder on the lane above the clip over the same span, unless "replace when done"
is chosen. Replacing SHALL change only the clip's media and SHALL keep its trim, speed, transform, effects and volume. An
audio result SHALL appear on an audio lane at the same span. Each placement or replacement SHALL be one revision.

#### Scenario: Upscale as a take
- **WHEN** a person upscales a clip without "replace when done"
- **THEN** a placeholder appears on the lane above over the same span, fills in when done, and the original clip is
  unchanged

#### Scenario: Replace keeps edits
- **WHEN** a reframe with "replace when done" finishes on a clip with speed 1.5 and a transform
- **THEN** the clip shows the new media with speed 1.5 and the same transform, in one revision

#### Scenario: Provider failure
- **WHEN** a lip-sync job fails at the provider
- **THEN** the placeholder is removed, the clip is unchanged, and the error is shown

### Requirement: Voice cloning consent
Cloning a voice SHALL require the person to confirm they have the right to use that voice. Without the confirmation the
clone SHALL NOT start.

#### Scenario: Clone without consent
- **WHEN** a voice clone is requested without the consent confirmation
- **THEN** the request is refused, and nothing is uploaded

### Requirement: AI transitions
A person or an agent SHALL be able to fill a gap of at most 15 s on a video track, or bridge a cut, with a clip generated
from the frame before and the frame after. The result SHALL fill the gap exactly, either by changing its speed or by
trimming it. If the gap is no longer empty when the job finishes, the result SHALL be placed as a take above.

#### Scenario: Gap filled by trim
- **WHEN** a transition with fit "trim" fills a 3.2 s gap and the model returns 4 s
- **THEN** the placed clip is 3.2 s long at speed 1

#### Scenario: Gap too long
- **WHEN** a transition is requested for a 20 s gap
- **THEN** the request is refused with a message about the 15 s limit

#### Scenario: Gap filled meanwhile
- **WHEN** a person drags a clip into the gap while the transition job runs
- **THEN** the result lands on the lane above, and the person's clip is unchanged

### Requirement: Action surfaces
Every AI action and transition SHALL be available from the command line and MCP with the same options as the app, and
the model list SHALL report catalog capabilities. Paid calls from agents SHALL follow the quote and confirmation rules.

#### Scenario: Command-line action
- **WHEN** `ove generate action upscale <project> --item i_ab12cd34 --json` runs and the person confirms the estimate
- **THEN** a take placeholder is created and the job id is returned with exit code 0

#### Scenario: Unknown item
- **WHEN** an MCP `ai_clip_action` call names an item id that does not exist
- **THEN** the tool returns an error naming the id, and nothing is spent
