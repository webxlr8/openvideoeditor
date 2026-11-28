# Spec Delta

## Purpose

The tools that agents use to read and edit a project are the same on every agent surface. These are the in-app
assistant, the MCP server and the pinned server for local agents. Edits made through these tools are attributed to the
agent that made them, can be undone without touching other people's work, and cannot spend money without a person's
confirmation.

## ADDED Requirements

### Requirement: One tool set on every agent surface
Each agent tool SHALL have one name, one description and one input schema. These SHALL be the same on every surface
where the tool appears. A tool that has the same name on two surfaces SHALL give the same result for the same input and
project state. The tool list SHALL be in a fixed order.

#### Scenario: Same result in-app and over MCP
- **WHEN** an agent calls `look` with the same arguments on the same project revision, once in-app and once over MCP
- **THEN** both calls return the same receipt fields and the same image bytes

#### Scenario: Stable tool list
- **WHEN** the tool list is requested twice from the same build
- **THEN** both lists are byte-identical, in the same order

#### Scenario: Unknown tool
- **WHEN** an agent calls a tool name that does not exist
- **THEN** the call fails with code `tool.unknown`, and the response lists the closest tool names

### Requirement: Text-edit apply on every surface
`apply_timeline` SHALL accept a list of text edits (old text and new text) together with the `rev` of the projection
that the agent read. It SHALL do this on every surface. Whole-file replacement SHALL remain available as a second mode.

#### Scenario: Edit over MCP without resending the file
- **WHEN** an MCP agent sends one text edit and the current `rev`
- **THEN** the edit is applied as one new revision, and the agent did not send the whole file

#### Scenario: Anchor not found
- **WHEN** an edit's old text does not appear in the projection of the given `rev`
- **THEN** nothing is applied, and the call fails with `edit.anchor` and names the edit by its index

### Requirement: Edits against an older revision are rebased
An edit made against an older `rev` SHALL be applied to the projection of that revision. It SHALL then be merged onto
the current document with the 3-way merge. It SHALL be refused only when the merge reports a conflict that the caller's
conflict preference does not resolve.

#### Scenario: Concurrent GUI edit
- **WHEN** a person moves item A in the editor after the agent printed the timeline, and the agent then edits item B
  against the older `rev`
- **THEN** both changes are present in the new revision, and the receipt reports no conflict

#### Scenario: Real conflict with editor preference
- **WHEN** the person and the agent changed the same field of the same item, and the agent asked to prefer the editor
- **THEN** the agent's change to that field is not applied, and the receipt lists the conflict with the item id and
  field

### Requirement: Edits to items deleted in the editor
With no `prefer`, an edit to an item the person deleted in the editor after the edit's `rev` SHALL fail with
`edit.conflict` and apply nothing. The message SHALL name each item by name and id and tell the agent to ask the
person whether to bring it back; the details SHALL list each item. The tool's text says to pass `prefer: "file"` only
after the person chooses that. This guard is advisory: the apply cannot check it.

#### Scenario: Edit to an item the person deleted, no preference
- **WHEN** the person deletes item A ("Intro", id `i_a`) in the editor after the agent printed the timeline, and the
  agent then edits item A against the older `rev` without `prefer`
- **THEN** nothing is applied, the call fails with `edit.conflict`, the message names "Intro" (`i_a`) and asks the agent
  to ask the person whether to bring it back, and the details list item A

#### Scenario: Edit to an item the person deleted, person chose to restore it
- **WHEN** the agent sends the same edit again with `prefer: "file"`
- **THEN** item A is re-created with its id and the edit, and the receipt lists the conflict

#### Scenario: The guard is advisory
- **WHEN** an agent passes `prefer: "file"` on its first apply, without asking the person
- **THEN** item A is re-created; the apply has no way to tell whether the person chose

### Requirement: The dry run agrees with the apply
`check_timeline` with no `prefer` SHALL report an edit to an item deleted in the editor as the same refusal that
`apply_timeline` gives: code `edit.conflict`, the same message and the same list of items. With `prefer: "file"` the
check SHALL pass and list the conflict. A check SHALL never change the project.

#### Scenario: Check without a preference
- **WHEN** the person deleted item A ("Intro", id `i_a`) in the editor, and an agent checks a text that edits item A
  without `prefer`
- **THEN** the check fails with `edit.conflict`, names "Intro" (`i_a`), and the details list item A

#### Scenario: Check with prefer file
- **WHEN** the agent checks the same text with `prefer: "file"`
- **THEN** the check passes, lists the conflict, and item A is still deleted

### Requirement: Apply error codes
A refused `apply_timeline` call SHALL fail with a code that names the reason. The codes include `argument.invalid`,
`edit.anchor`, `edit.ambiguous`, `edit.overlap`, `edit.empty`, `edit.conflict`, `rev.malformed`, `rev.pruned`,
`rev.newer`, `check.failed` (with the diagnostics) and `project.locked`.

#### Scenario: Refusal carries its code
- **WHEN** an apply is refused because an edit changes an item deleted in the editor and no `prefer` was given
- **THEN** the error code is `edit.conflict`, not a generic failure

### Requirement: Edit receipts
Every successful edit SHALL return a receipt with these fields:
- the new `rev`;
- for each added or changed item, its id and its printed line;
- the id and name of each removed item;
- for each group of three or more items on one track that moved by the same amount, one shift rule with track, start
  time, delta and count;
- whether track order changed.

At most 30 changed items SHALL be listed. When there are more, the receipt SHALL give the count of the rest.

#### Scenario: New item id returned
- **WHEN** an agent adds a text item without an id
- **THEN** the receipt contains the id assigned to it and its printed line, and that line can be used as the old text of
  the next edit

#### Scenario: Shift compressed
- **WHEN** an edit moves 12 items on one track by 2 seconds
- **THEN** the receipt has one shift rule with count 12, and does not list the 12 items one by one

### Requirement: Actor attribution
Each new revision SHALL record its actor: the kind (person, assistant, CLI, MCP, template or generation), and the
session, turn and client name when these are known. Revisions written before this capability SHALL open with no actor
and SHALL keep working.

#### Scenario: Two MCP sessions told apart
- **WHEN** two MCP sessions with different client names each apply an edit to the same project
- **THEN** history shows two revisions with different sessions and client names

#### Scenario: Old bundle
- **WHEN** a project saved by the previous release is opened
- **THEN** its history opens, the older revisions show no actor, and new revisions record one

### Requirement: Undo only your own changes
For agents, `undo_timeline` SHALL undo only the most recent revision made by the calling session. `undo_my_changes`
SHALL undo all revisions of the calling session, or of one turn. Both SHALL create one new revision that inverts only
those revisions. When a later revision by another actor changed the same fields, both SHALL refuse and list the
conflicting items. Undoing another actor's revisions SHALL NOT be possible through agent tools.

#### Scenario: Person's edits survive
- **WHEN** the person edits, then an MCP agent edits, then the person edits again, and the agent calls `undo_timeline`
- **THEN** only the agent's change is reverted, and both of the person's edits remain

#### Scenario: Conflict refused
- **WHEN** the agent asks to undo its change to an item's start time, and the person has since changed that start time
- **THEN** nothing changes, and the call fails with `undo.conflict` and lists the item

### Requirement: Session project binding
An MCP session started for one project SHALL apply every project tool to that project, and SHALL refuse calls that name
another project. An unbound session SHALL be able to set a default project, after which `project` is optional. Writes
SHALL be accepted when the project is open in the app but not in front.

#### Scenario: Pinned session
- **WHEN** a session started for project P calls `apply_timeline` and names project Q
- **THEN** the call fails with `project.pinned`, and Q is unchanged

#### Scenario: Project not in front
- **WHEN** a session edits project P while the person is working in project Q's window
- **THEN** the edit is applied to P as a new revision and appears in P's history

### Requirement: Coordinate grid on inspected frames
`look` and `grab_frame` SHALL accept a grid option. With the grid on, the returned image SHALL carry:
- a labelled grid in the same units that item transforms use;
- the title-safe and action-safe boxes.

The grid SHALL never appear in exported video.

#### Scenario: Grid units match transforms
- **WHEN** a text item has transform x of −320 and the agent requests a grid frame
- **THEN** the item's centre appears at the grid line labelled −320

#### Scenario: Export unaffected
- **WHEN** a project is exported after grid frames were requested in the same session
- **THEN** the exported frames contain no grid

### Requirement: Storyboard sheets
`filmstrip` and `look` SHALL offer a storyboard mode. It SHALL return a sheet of at most 36 tiles that leaves out
near-duplicate frames. Each tile SHALL have its timecode burned in. On timeline sheets, each tile SHALL also show the ids
of the items visible at that time.

#### Scenario: Cuts covered
- **WHEN** a storyboard is requested for a 10-minute clip
- **THEN** the sheet has at most 36 tiles, and at least one tile falls inside each shot that scene detection reports

### Requirement: Analysis tools for the in-app assistant
The in-app assistant SHALL be able to call the same frame and analysis tools as MCP agents. These are frames, filmstrip
and storyboard sheets, scene cuts, audio levels and silence detection.

#### Scenario: Assistant looks at a frame
- **WHEN** the person asks the in-app assistant to check the title position
- **THEN** the assistant can request a composited frame of the timeline and receives the image in its turn

### Requirement: Generation tools need a person's confirmation
Generation tools SHALL first return a plan, an estimated cost and a quote id, and SHALL spend nothing at that step. A
call that spends SHALL succeed only after a person approved that quote in the app, through the client's confirmation
prompt, or in the in-app question card. A quote SHALL expire after 10 minutes. A quote SHALL be valid only for the exact
request it priced. A session SHALL be refused once its total confirmed spend would exceed the session cap.

#### Scenario: Quote then confirm
- **WHEN** an agent requests a video and the person approves the price card
- **THEN** the job starts, and a pending placeholder appears on the timeline as one revision that can be undone

#### Scenario: Agent confirms alone
- **WHEN** an agent passes the quote id as confirmation without any approval by a person
- **THEN** nothing is spent, and the call fails with `confirm.required`

#### Scenario: Headless client without confirmation support
- **WHEN** a client that cannot show a confirmation prompt requests a generation and the app is not running
- **THEN** the call fails with `confirm.required` and explains that the app or a capable client is needed

#### Scenario: Over the cap
- **WHEN** a confirmed request would take the session's spend above its cap
- **THEN** nothing is spent, and the call fails with `spend.cap`

### Requirement: Local feedback queue
`send_feedback` SHALL store a report on this computer only. The report SHALL contain the category, severity, a
paraphrased summary, the app and OS versions and the recent tool names. The tool SHALL drop exact duplicates and accept
at most 8 reports per session. Help → Feedback SHALL list the queued reports so that the person can edit, delete or copy
each one. No report SHALL be sent from the app.

#### Scenario: No network
- **WHEN** an agent calls `send_feedback`
- **THEN** a report appears in Help → Feedback, and no network request is made

#### Scenario: Limit
- **WHEN** an agent calls `send_feedback` a ninth time in one session
- **THEN** the call is refused with `feedback.limit`, and no new report is stored
