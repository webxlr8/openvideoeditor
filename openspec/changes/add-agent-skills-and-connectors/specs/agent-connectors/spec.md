# Spec Delta

## Purpose

People connect the local agent clients they already use to the editor in one step, on macOS and Windows, with a manual
fallback.

## ADDED Requirements

### Requirement: Claude Desktop connection
Settings → Agents SHALL offer to connect Claude Desktop when it is installed. Connecting SHALL install an extension that
starts the editor's own command-line tool as an MCP server. It SHALL need no other runtime. A copyable manual
configuration SHALL be shown next to the button.

#### Scenario: Connect
- **WHEN** the person clicks Connect for Claude Desktop on macOS or Windows
- **THEN** Claude Desktop shows the extension install prompt, and after install its tool list contains the editor's tools

#### Scenario: Claude Desktop not installed
- **WHEN** Claude Desktop is not detected
- **THEN** the row shows "not installed" with a link to install it, and the manual configuration is still shown

#### Scenario: Install refused by the client
- **WHEN** Claude Desktop refuses the extension
- **THEN** the row shows the manual configuration steps and a copy button

### Requirement: Cursor connection
Settings → Agents SHALL offer to connect Cursor when it is installed. Connecting SHALL open an install link that adds
the editor's MCP server with the absolute path of the command-line tool. A copyable configuration snippet SHALL be shown
next to the button.

#### Scenario: Connect
- **WHEN** the person clicks Connect for Cursor
- **THEN** Cursor opens its MCP install prompt with the editor's server prefilled

#### Scenario: Link not handled
- **WHEN** the OS has no handler for the install link
- **THEN** the person sees the configuration snippet with the file location to paste it into

### Requirement: Connection status
Each connector row SHALL show whether that client is installed and whether it is connected. A client is connected when
its configuration contains the editor's server.

#### Scenario: Already connected
- **WHEN** Cursor's configuration already contains the editor's server
- **THEN** the Cursor row shows Connected and offers Disconnect
