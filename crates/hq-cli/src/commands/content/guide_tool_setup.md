---
noteType: guide
fileName: tool-setup
version: 1
---
# Tool Setup Guide

This guide covers installing and configuring the tools that extend HQ's capabilities. Run `hq onboard` for an interactive walkthrough, or follow these manual steps.

## Google Workspace CLI (gws)

The `gws` CLI gives HQ agents access to Google Drive, Gmail, Calendar, and Sheets. This is one of the most impactful integrations: it enables calendar-aware briefs, email triage, and document management.

### Install

```bash
# macOS (Homebrew)
brew install nicholasgasior/tap/gws

# Or download the binary from:
# https://github.com/nicholasgasior/gws/releases
```

### Authenticate

```bash
gws auth login
```

This opens a browser for Google OAuth. Grant access to the requested scopes (Drive, Gmail, Calendar, Sheets).

### Verify

```bash
gws calendar +agenda         # Show upcoming events
gws gmail +triage            # Show unread email summary
gws drive files list --params '{"q": "trashed = false", "pageSize": 5}'
```

### What It Enables

- **Email triage**: Agents can scan, summarize, and draft replies to your email.
- **Drive management**: Upload files, organize folders, search documents.
- **Spreadsheet access**: Read and write Google Sheets data.

### Quick Reference

```bash
# Drive
gws drive files list --params '{"q": "name contains '\''report'\''", "fields": "files(id,name)"}'
gws drive files create --json '{"name": "file.pdf", "parents": ["FOLDER_ID"]}' --upload /path/to/file.pdf

# Sheets
gws sheets +read --params '{"spreadsheetId": "SHEET_ID", "range": "Sheet1!A1:D10"}'
gws sheets +append --params '{"spreadsheetId": "SHEET_ID", "range": "Sheet1"}' --json '{"values": [["a","b"]]}'

# Gmail
gws gmail +send --params '{"to": "x@y.com", "subject": "Hello"}' --json '{"body": "Message body"}'

# Calendar
gws calendar +insert --params '{"calendarId": "primary"}' --json '{"summary": "Meeting", "start": {"dateTime": "2026-03-20T10:00:00Z"}, "end": {"dateTime": "2026-03-20T11:00:00Z"}}'
```

## MCP Server (Claude Desktop / VS Code)

The MCP server connects HQ's 51 tools to Claude Desktop or VS Code editors.

### Install

```bash
hq mcp install
```

This adds `agent-hq` as an MCP server in your Claude Desktop config. After restarting Claude Desktop, you'll have access to all HQ tools (vault operations, search, jobs, diagrams, and more).

### Verify

```bash
hq mcp status
```

## DrawIt CLI (Diagrams)

Generate Mermaid and SVG diagrams from natural language.

### Install

```bash
npm install -g @chamuka-labs/drawit-cli
```

### Verify

```bash
drawit --version
```
