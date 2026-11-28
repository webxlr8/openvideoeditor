# Spec Delta

## Purpose

The person points the assistant at things: media, timeline items and time ranges. Panels hand prepared requests to the
chat. The chat shows streamed reasoning without clutter.

## ADDED Requirements

### Requirement: Mentions of media, items and ranges
Typing @ in the chat input SHALL offer media, timeline items and the selected range. A sent mention SHALL give the
assistant:
- the exact projection text of the mentioned items;
- the revision that the text came from.

Mentioned images SHALL be attached as images. Mentions SHALL be saved with the chat. They SHALL appear the same way when
the chat is reopened.

#### Scenario: Edit without printing
- **WHEN** the person mentions a title item and asks the assistant to move it up
- **THEN** the assistant edits the item in that turn without printing the timeline first

#### Scenario: Item changed after sending
- **WHEN** the person moves the mentioned item before the assistant's edit arrives
- **THEN** the assistant's edit is merged onto the moved item, and no edit is lost

#### Scenario: Mentioned item deleted
- **WHEN** the mentioned item was deleted before the assistant edits it
- **THEN** the assistant's edit is reported as a conflict, and the deleted item is not created again without the
  person's choice

#### Scenario: Too many images
- **WHEN** the person mentions more than 5 images in one message
- **THEN** only 5 are attached, and the chat shows a notice that names the images that were left out

### Requirement: Panel hand-off
The Captions, Audio, Media and Effects panels SHALL be able to open the chat with a prepared request and mentions. The
request SHALL be placed in the input for the person to edit. It SHALL NOT be sent automatically.

#### Scenario: Remove fillers from captions
- **WHEN** the person chooses "Remove filler words" in the Captions panel
- **THEN** the chat opens with that request and a mention of the caption track in the input, and nothing is sent until
  the person sends it

### Requirement: Skills and templates menu
Typing `/` at the start of the chat input SHALL list installed skills and project templates, filtered by what follows
the slash. Choosing a skill SHALL make the assistant read that skill before it acts. Choosing a template SHALL insert
its request, and SHALL open its intake form when it has one.

#### Scenario: Use a skill
- **WHEN** the person picks the colour matching skill from the `/` menu and sends "match these two shots"
- **THEN** the assistant reads that skill at the start of the turn

#### Scenario: No match
- **WHEN** the text after the slash matches no skill or template
- **THEN** the menu shows "No matches", and the text is sent as typed if the person presses Enter

### Requirement: Collapsible streamed reasoning
When the provider streams reasoning, the chat SHALL show it in a collapsed block that can be expanded. The block SHALL
show the elapsed time. Streaming SHALL NOT make the chat jump or stutter.

#### Scenario: Expand reasoning
- **WHEN** a turn streams reasoning and the person expands the block
- **THEN** the reasoning text so far is shown and continues to update
