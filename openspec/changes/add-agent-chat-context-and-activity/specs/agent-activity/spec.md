# Spec Delta

## Purpose

The person can see which agent is reading or changing which part of the timeline, and undo one agent's changes from
History.

## ADDED Requirements

### Requirement: Live activity on the timeline
While an agent reads or changes the open project, the timeline SHALL show what it is doing:
- the time range it reads, as a band on the ruler;
- the items it reads or changes, as outlines;
- the agent's name, as a badge.

This SHALL apply to the in-app assistant and to external agents that work through the running app. The display SHALL
fade after the activity stops.

#### Scenario: External agent edit
- **WHEN** an external agent applies an edit to the open project through the running app
- **THEN** the changed items are outlined within 300 ms, and a badge shows the client's name

#### Scenario: Agent reads a range
- **WHEN** the in-app assistant requests frames between 10 s and 20 s
- **THEN** the ruler shows a band from 10 s to 20 s while the request runs

#### Scenario: Person edits at the same time
- **WHEN** the person edits an item while an agent's change is being applied
- **THEN** the person's edit is not outlined as an agent change

#### Scenario: Flood of edits
- **WHEN** an agent applies 1,000 edits in 10 seconds
- **THEN** the timeline stays responsive, and at most 200 items are outlined at once, with a count of the rest

### Requirement: History grouped by actor
The History panel SHALL group consecutive revisions by actor and show each actor's name. An agent group SHALL offer
"Undo this agent's changes". That action SHALL undo only that agent's revisions as one new revision. It SHALL refuse,
listing the conflicting items, when later edits by others changed the same fields.

#### Scenario: Undo one agent
- **WHEN** Claude Code and the person both edited, and the person chooses "Undo this agent's changes" on the Claude Code
  group
- **THEN** only the Claude Code changes are reverted, and the person's edits remain

#### Scenario: Conflict
- **WHEN** the person has since changed a field that the agent changed
- **THEN** nothing changes, and the panel lists the conflicting items
