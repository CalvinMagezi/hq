# HQ Control Center

PWA for Agent-HQ: vault, chat, tasks and notifications. Built with TanStack Start (SPA mode), React 19 and Vite.

## Architecture

The app builds to a static single-page app (`dist/client/index.html` plus assets and `sw.js`). The Rust server (`hq start all`, port 5678) serves it and is its only backend: REST under `/api`, streaming chat and live updates over `/ws`. There is no Node or Bun process at runtime. All requests go through `src/lib/hqAuth.ts`, which adds the web token as an Authorization header when the server sets `web_auth_token`. The chat socket opens with a single-use ticket and vault files load through `src/lib/useAssetUrl.ts`, so the token never appears in a URL (see `docs/security/WEB_AUTH.md`). Vault calls live in `src/lib/vaultApi.ts`.

For HTTPS over Tailscale, see `deploy/Caddyfile.pwa` and `deploy/README.md`.

## Development

```bash
bun install
bun run dev          # dev server on :4747, proxies /api and /ws to hq on :5678
```

## Build

```bash
bun run build        # dist/client: point hq at it with web_static_dir, or copy it to web/dist next to the vault
```

## Key Dependencies

| Package | Purpose |
|---------|---------|
| @tanstack/react-start | App framework, built in SPA mode |
| @tanstack/react-query | Data fetching |
| @tanstack/react-router | File-based routing |
| zustand | State management |
| framer-motion | Animations |
| shiki | Code syntax highlighting |
| recharts | Charts and graphs |
| marked | Markdown rendering |
| mammoth | DOCX rendering |
| xlsx | Spreadsheet rendering |
| pdfjs-dist | PDF rendering |
| dompurify | HTML sanitization |
| tailwindcss v4 | Styling |

## WebSocket events

`src/context/WebSocketContext.tsx` handles the chat stream (`turn_start`, `text_delta`, `reasoning_delta`, `tool_start`, `tool_progress`, `tool_end`, `error`, `turn_end`, `thread_title`) and live updates for notifications, tasks and system notices. The client sends `chat` and `stop`.

## API used by the app

| Area | Endpoints |
|------|-----------|
| Chat | `/api/threads`, `/api/threads/{id}/messages`, `/api/threads/{id}/archive` |
| Vault | `/api/tree?recursive=true`, `/api/note` (GET, PUT), `/api/note/create`, `/api/pinned`, `/api/pin`, `/api/vault-signals`, `/api/vault/folders`, `/api/search`, `/api/vault-asset`, `/api/vault-status` |
| Tasks | `/api/spaces`, `/api/folders`, `/api/initiatives`, `/api/tasks`, `/api/tasks/{id}` (PATCH, DELETE), `/api/tasks/{id}/comments` |
| Inbox | `/api/notifications`, `/api/notifications/{id}/action`, `/api/notifications/mark-all-read` |
